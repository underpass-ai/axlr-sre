use serde_json::Value;
use sre_mcp::ToolDefinition;

const NAMESPACE: &str =
    "Namespace; the first of KUBE_NAMESPACES when omitted. Must be in the server's allowlist";
const ROLLOUT_KINDS: &[&str] = &["deployment", "statefulset", "daemonset"];
const SCALE_KINDS: &[&str] = &["deployment", "statefulset", "replicaset"];
const CONFIRM: &str = "false previews and changes nothing; true runs it";
const MANIFEST: &str = "The manifest, YAML or JSON, one or more documents, every object in the call's namespace. Never a Secret, Namespace, PVC, CRD, Node, RBAC, NetworkPolicy or admission webhook";

/// The read tools every server publishes.
fn reads() -> Vec<ToolDefinition> {
    vec![
        ToolDefinition::doctor(),
        ToolDefinition::discover().text("namespace", NAMESPACE),
        ToolDefinition::new("get", "Lists resources of a kind, or one by name, summarised: name, status, readiness, restarts, replicas. raw gives the whole objects")
            .text("kind", "Resource kind as kubectl names it: pods, deployments, hpa, ingresses.networking.k8s.io. Never secrets")
            .text("name", "One resource by exact name")
            .text("namespace", NAMESPACE)
            .text("selector", "Label selector, such as app=web")
            .number("limit", "Items at most, 1 to 200", 50)
            .flag("raw", "The whole objects without managedFields instead of summaries", false)
            .required(&["kind"]),
        ToolDefinition::new("describe", "kubectl describe of one resource: its spec, status, conditions and recent events, 64 KiB at most")
            .text("kind", "Resource kind as kubectl names it. Never secrets")
            .text("name", "The exact resource name")
            .text("namespace", NAMESPACE)
            .required(&["kind", "name"]),
        ToolDefinition::new("logs", "The last lines of a pod's container, 256 KiB at most")
            .text("pod", "The exact pod name")
            .text("namespace", NAMESPACE)
            .text("container", "Only this container; the first one when omitted")
            .number("tail", "Last lines, 1 to 1000", 200)
            .text("since", "Only lines newer than this, such as 30s, 5m or 2h")
            .flag("previous", "The previous, crashed, instance of the container", false)
            .required(&["pod"]),
        ToolDefinition::new("events", "The namespace's events, newest first: type, reason, object, message, count")
            .text("namespace", NAMESPACE)
            .text("object", "Only events about this object, by exact name")
            .flag("warnings_only", "Only Warning events", true)
            .number("limit", "Events at most, 1 to 200", 50),
        ToolDefinition::new("rollout_status", "Waits for a rollout: complete, or timed_out with what is still rolling")
            .choice("kind", "What is rolling", ROLLOUT_KINDS, "deployment")
            .text("name", "The exact workload name")
            .text("namespace", NAMESPACE)
            .number("timeout_seconds", "How long to wait, 1 to 600", 120)
            .required(&["name"]),
        ToolDefinition::new("rollout_history", "The revisions of a workload, for undo")
            .choice("kind", "The workload kind", ROLLOUT_KINDS, "deployment")
            .text("name", "The exact workload name")
            .text("namespace", NAMESPACE)
            .required(&["name"]),
    ]
}

/// The write tools, published only when the server writes.
fn writes() -> Vec<ToolDefinition> {
    vec![
        ToolDefinition::new("diff", "What applying the manifest would change: differences true or false, and the diff. A server dry run; the cluster is not changed")
            .text("manifest", MANIFEST)
            .text("namespace", NAMESPACE)
            .required(&["manifest"]),
        ToolDefinition::new("apply", "Applies the manifest after a diff and a server dry run. Without confirm, the preview: the diff and what the dry run accepted")
            .text("manifest", MANIFEST)
            .text("namespace", NAMESPACE)
            .flag("confirm", CONFIRM, false)
            .required(&["manifest"]),
        ToolDefinition::new("rollout_restart", "Restarts a workload's pods, rolling. Without confirm, the preview: the workload as it is now and the command")
            .choice("kind", "The workload kind", ROLLOUT_KINDS, "deployment")
            .text("name", "The exact workload name")
            .text("namespace", NAMESPACE)
            .flag("confirm", CONFIRM, false)
            .required(&["name"]),
        ToolDefinition::new("rollout_undo", "Rolls a workload back to its previous revision, or to one named. Without confirm, the preview")
            .choice("kind", "The workload kind", ROLLOUT_KINDS, "deployment")
            .text("name", "The exact workload name")
            .text("namespace", NAMESPACE)
            .number("revision", "The revision to go back to; the previous one when omitted", 0)
            .flag("confirm", CONFIRM, false)
            .required(&["name"]),
        ToolDefinition::new("scale", "Sets a workload's replicas, 0 to KUBE_MAX_REPLICAS. Without confirm, the preview: the current count and the command")
            .choice("kind", "The workload kind", SCALE_KINDS, "deployment")
            .text("name", "The exact workload name")
            .number("replicas", "The replicas wanted", 1)
            .text("namespace", NAMESPACE)
            .flag("confirm", CONFIRM, false)
            .required(&["name", "replicas"]),
    ]
}

pub(crate) const WRITE_TOOLS: &[&str] =
    &["diff", "apply", "rollout_restart", "rollout_undo", "scale"];

/// The catalogue `tools/list` publishes: the reads, and the writes when enabled.
pub(crate) fn tools(with_writes: bool) -> Vec<Value> {
    let mut all = reads();
    if with_writes {
        all.extend(writes());
    }
    all.iter().map(ToolDefinition::to_json).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_read_only_server_publishes_eight_tools_and_a_writer_thirteen() {
        let reads = tools(false);
        assert_eq!(reads.len(), 8);
        assert_eq!(reads[0]["name"], "doctor");
        assert_eq!(reads[1]["name"], "discover");
        for tool in &reads {
            assert!(!WRITE_TOOLS.contains(&tool["name"].as_str().unwrap()));
        }
        let all = tools(true);
        assert_eq!(all.len(), 13);
        let names: Vec<&str> = all.iter().map(|t| t["name"].as_str().unwrap()).collect();
        for write in WRITE_TOOLS {
            assert!(names.contains(write), "{write}");
        }
    }

    #[test]
    fn every_write_previews_by_default_but_diff() {
        for tool in tools(true)
            .iter()
            .filter(|t| WRITE_TOOLS.contains(&t["name"].as_str().unwrap()) && t["name"] != "diff")
        {
            assert_eq!(
                tool["inputSchema"]["properties"]["confirm"]["default"], false,
                "{}",
                tool["name"]
            );
        }
    }

    #[test]
    fn no_tool_takes_every_namespace() {
        for tool in tools(true) {
            assert!(
                tool["inputSchema"]["properties"]
                    .get("all_namespaces")
                    .is_none()
            );
            assert_eq!(tool["inputSchema"]["additionalProperties"], false);
        }
    }
}
