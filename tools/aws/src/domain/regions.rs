use super::{ConfigError, InputError, region};

/// The regions the server may look at: an allowlist whose first entry is
/// the default, or no list, in which case every call names its region. The
/// CLI's own default region is never used: an ambient `AWS_DEFAULT_REGION`
/// on the node would otherwise decide where a bot looks.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Regions {
    allowed: Vec<String>,
}

impl Regions {
    pub fn new(raw: Option<&str>) -> Result<Self, ConfigError> {
        let mut allowed = Vec::new();
        for entry in raw.unwrap_or_default().split(',') {
            let entry = entry.trim();
            if entry.is_empty() {
                continue;
            }
            let region = region(entry)
                .map_err(|_| ConfigError::InvalidRegions(entry.chars().take(60).collect()))?;
            if !allowed.contains(&region) {
                allowed.push(region);
            }
        }
        Ok(Self { allowed })
    }

    #[must_use]
    pub fn allowed(&self) -> &[String] {
        &self.allowed
    }

    #[must_use]
    pub fn default_region(&self) -> Option<&str> {
        self.allowed.first().map(String::as_str)
    }

    /// The region a call runs in: the one named, checked against the list,
    /// or the list's first when none is named.
    pub fn resolve(&self, requested: Option<&str>) -> Result<String, InputError> {
        match requested.map(str::trim).filter(|r| !r.is_empty()) {
            Some(name) => {
                let region = region(name)?;
                if self.allowed.is_empty() || self.allowed.contains(&region) {
                    Ok(region)
                } else {
                    Err(InputError::RegionNotAllowed(region))
                }
            }
            None => self
                .default_region()
                .map(str::to_string)
                .ok_or(InputError::RegionRequired),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_first_region_is_the_default_and_others_are_checked() {
        let regions = Regions::new(Some(" eu-west-1, us-east-1 ,eu-west-1")).unwrap();
        assert_eq!(regions.allowed(), ["eu-west-1", "us-east-1"]);
        assert_eq!(regions.default_region(), Some("eu-west-1"));
        assert_eq!(regions.resolve(None).unwrap(), "eu-west-1");
        assert_eq!(regions.resolve(Some("US-EAST-1")).unwrap(), "us-east-1");
        assert_eq!(
            regions.resolve(Some("ap-south-1")).unwrap_err(),
            InputError::RegionNotAllowed("ap-south-1".into())
        );
        assert!(matches!(
            regions.resolve(Some("mars")).unwrap_err(),
            InputError::InvalidRegion(_)
        ));
    }

    #[test]
    fn without_a_list_every_call_names_its_region() {
        let any = Regions::new(None).unwrap();
        assert_eq!(any.default_region(), None);
        assert_eq!(any.resolve(None).unwrap_err(), InputError::RegionRequired);
        assert_eq!(any.resolve(Some("sa-east-1")).unwrap(), "sa-east-1");
        assert_eq!(Regions::new(Some(" , ")).unwrap(), Regions::default());
        assert_eq!(
            Regions::new(Some("eu-west-1,nowhere")).unwrap_err(),
            ConfigError::InvalidRegions("nowhere".into())
        );
    }
}
