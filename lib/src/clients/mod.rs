pub mod bundle;
pub mod capabilities;
pub mod drs;
pub mod pagination;
pub mod registry;
pub mod serviceinfo;
pub mod tes;
pub mod trs;
pub mod wait;
pub mod wes;

/// A section of the configuration file, named after the service it configures.
///
/// `Custom` names the section of any other service, for example one that a
/// downstream crate adds. More variants may be added, so a `match` needs a
/// wildcard arm.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum ServiceType {
    TES,
    DRS,
    TRS,
    AAI,
    WES,
    REGISTRY,
    /// Another service, by the name of its section in the configuration file.
    ///
    /// `Custom("TES")` reads the same section as `TES`, but the two don't compare
    /// equal; use the built-in variants for built-in services.
    Custom(String),
}

impl ServiceType {
    /// Returns the name of the service's section in the configuration file.
    pub fn as_str(&self) -> &str {
        match self {
            ServiceType::TES => "TES",
            ServiceType::DRS => "DRS",
            ServiceType::TRS => "TRS",
            ServiceType::AAI => "AAI",
            ServiceType::WES => "WES",
            ServiceType::REGISTRY => "REGISTRY",
            ServiceType::Custom(name) => name,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::ServiceType;

    #[test]
    fn test_service_type_section_names() {
        assert_eq!(ServiceType::TES.as_str(), "TES");
        assert_eq!(ServiceType::REGISTRY.as_str(), "REGISTRY");
        assert_eq!(ServiceType::Custom("PROXY".to_owned()).as_str(), "PROXY");
    }

    #[test]
    fn test_custom_service_type_differs_from_built_in() {
        assert_ne!(ServiceType::Custom("TES".to_owned()), ServiceType::TES);
        assert_eq!(
            ServiceType::Custom("PROXY".to_owned()),
            ServiceType::Custom("PROXY".to_owned())
        );
    }
}
