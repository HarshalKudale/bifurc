use gpui_kit::component::IconName;

/// What the rail switches the explorer between.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Module {
    Requests,
    Environments,
    Mappings,
}

impl Module {
    pub const ALL: [Module; 3] = [Module::Requests, Module::Environments, Module::Mappings];

    pub fn label(self) -> &'static str {
        match self {
            Module::Requests => "Requests",
            Module::Environments => "Environments",
            Module::Mappings => "Mappings",
        }
    }

    pub fn icon(self) -> IconName {
        match self {
            Module::Requests => IconName::Inbox,
            Module::Environments => IconName::Globe,
            Module::Mappings => IconName::Replace,
        }
    }
}
