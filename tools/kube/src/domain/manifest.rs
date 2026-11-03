use serde::Deserialize;
use serde_json::Value;

use super::{InputError, tokens};

const MANIFEST_MAX: usize = 262_144;

/// Kinds this server never writes, whatever the mode: the blast-radius list
/// of the design (§8.1) plus RBAC, network policy and admission.
const FORBIDDEN_KINDS: &[&str] = &[
    "secret",
    "namespace",
    "persistentvolumeclaim",
    "persistentvolume",
    "customresourcedefinition",
    "node",
    "role",
    "clusterrole",
    "rolebinding",
    "clusterrolebinding",
    "networkpolicy",
    "mutatingwebhookconfiguration",
    "validatingwebhookconfiguration",
    "validatingadmissionpolicy",
    "validatingadmissionpolicybinding",
];

/// One object of a manifest, as much of it as the policy needs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ManifestObject {
    pub kind: String,
    pub name: String,
}

impl ManifestObject {
    #[must_use]
    pub fn label(&self) -> String {
        format!("{}/{}", self.kind, self.name)
    }
}

/// A manifest a write tool received: YAML or JSON, one or more documents,
/// each a namespaced object of an allowed kind in the namespace of the call.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Manifest {
    text: String,
    objects: Vec<ManifestObject>,
}

impl Manifest {
    pub fn parse(text: &str, namespace: &str) -> Result<Self, InputError> {
        if text.len() > MANIFEST_MAX {
            return Err(InputError::ManifestTooLarge(text.len()));
        }
        let mut objects = Vec::new();
        for document in serde_yaml_ng::Deserializer::from_str(text) {
            let value = Value::deserialize(document)
                .map_err(|error| InputError::InvalidManifest(error.to_string()))?;
            if value.is_null() {
                continue;
            }
            objects.push(object_of(&value, namespace)?);
        }
        if objects.is_empty() {
            return Err(InputError::EmptyManifest);
        }
        Ok(Self {
            text: text.to_string(),
            objects,
        })
    }

    #[must_use]
    pub fn text(&self) -> &str {
        &self.text
    }

    #[must_use]
    pub fn objects(&self) -> &[ManifestObject] {
        &self.objects
    }

    #[must_use]
    pub fn labels(&self) -> Vec<String> {
        self.objects.iter().map(ManifestObject::label).collect()
    }
}

fn object_of(value: &Value, namespace: &str) -> Result<ManifestObject, InputError> {
    let Some(kind) = value.get("kind").and_then(Value::as_str) else {
        return Err(InputError::InvalidManifest(
            "a document has no kind".to_string(),
        ));
    };
    if kind == "List" {
        return Err(InputError::InvalidManifest(
            "send the objects as documents separated by ---, not as a List".to_string(),
        ));
    }
    if FORBIDDEN_KINDS.contains(&kind.to_ascii_lowercase().as_str()) {
        return Err(InputError::ForbiddenKind(kind.to_string()));
    }
    let metadata = value.get("metadata").unwrap_or(&Value::Null);
    let name = metadata
        .get("name")
        .and_then(Value::as_str)
        .ok_or_else(|| InputError::InvalidManifest(format!("a {kind} has no metadata.name")))?;
    let name = tokens::name(name)?;
    if let Some(own) = metadata.get("namespace").and_then(Value::as_str)
        && own != namespace
    {
        return Err(InputError::ForeignNamespace {
            kind: kind.to_string(),
            name,
            namespace: own.to_string(),
        });
    }
    Ok(ManifestObject {
        kind: kind.to_string(),
        name,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const TWO: &str = "apiVersion: apps/v1\nkind: Deployment\nmetadata:\n  name: web\n  namespace: prod\nspec: {replicas: 2}\n---\napiVersion: v1\nkind: ConfigMap\nmetadata: {name: web-config}\ndata: {a: '1'}\n";

    #[test]
    fn documents_in_the_namespace_are_listed() {
        let manifest = Manifest::parse(TWO, "prod").unwrap();
        assert_eq!(
            manifest.labels(),
            ["Deployment/web", "ConfigMap/web-config"]
        );
        assert_eq!(manifest.text(), TWO);
        assert_eq!(manifest.objects()[0].kind, "Deployment");
    }

    #[test]
    fn json_is_a_manifest_too_and_empty_documents_are_skipped() {
        let manifest = Manifest::parse(
            "---\n{\"kind\": \"Service\", \"metadata\": {\"name\": \"web\"}}\n---\n",
            "prod",
        )
        .unwrap();
        assert_eq!(manifest.labels(), ["Service/web"]);
    }

    #[test]
    fn a_foreign_namespace_is_refused() {
        assert_eq!(
            Manifest::parse(TWO, "staging"),
            Err(InputError::ForeignNamespace {
                kind: "Deployment".into(),
                name: "web".into(),
                namespace: "prod".into()
            })
        );
    }

    #[test]
    fn the_blast_radius_kinds_are_refused_whatever_the_case() {
        for kind in [
            "Secret",
            "secret",
            "Namespace",
            "PersistentVolumeClaim",
            "CustomResourceDefinition",
            "Node",
            "ClusterRoleBinding",
            "NetworkPolicy",
            "MutatingWebhookConfiguration",
        ] {
            let text = format!("kind: {kind}\nmetadata: {{name: x}}\n");
            assert_eq!(
                Manifest::parse(&text, "prod"),
                Err(InputError::ForbiddenKind(kind.into())),
                "{kind}"
            );
        }
    }

    #[test]
    fn malformed_manifests_say_what_is_missing() {
        assert_eq!(Manifest::parse("", "prod"), Err(InputError::EmptyManifest));
        assert_eq!(
            Manifest::parse("---\n", "prod"),
            Err(InputError::EmptyManifest)
        );
        assert!(
            matches!(Manifest::parse("metadata: {name: x}", "prod"), Err(InputError::InvalidManifest(m)) if m.contains("kind"))
        );
        assert!(
            matches!(Manifest::parse("kind: Pod", "prod"), Err(InputError::InvalidManifest(m)) if m.contains("metadata.name"))
        );
        assert!(
            matches!(Manifest::parse("kind: List\nitems: []", "prod"), Err(InputError::InvalidManifest(m)) if m.contains("List"))
        );
        assert!(matches!(
            Manifest::parse("kind: Pod\nmetadata: {name: Bad}", "prod"),
            Err(InputError::InvalidName(_))
        ));
        assert!(matches!(
            Manifest::parse("a: [", "prod"),
            Err(InputError::InvalidManifest(_))
        ));
        assert!(matches!(
            Manifest::parse("just a string", "prod"),
            Err(InputError::InvalidManifest(_))
        ));
        let big = format!(
            "kind: Pod\nmetadata: {{name: x}}\n# {}",
            "x".repeat(MANIFEST_MAX)
        );
        assert!(matches!(
            Manifest::parse(&big, "prod"),
            Err(InputError::ManifestTooLarge(_))
        ));
    }
}
