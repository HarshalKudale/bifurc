//! Service discovery (`services.discover`), the health bar (`healthbar.*`) and dev servers (`devservice.*`).

use serde_json::Value;

use crate::kernel::CommandRegistry;

pub mod dev;
pub mod discovery;
pub mod healthbar;

pub fn register(registry: &mut CommandRegistry) {
    registry.register("services.discover", |_, _| async move {
        Ok(Value::Array(discovery::discover_services().await.iter().map(discovery::ServiceInfo::to_json).collect()))
    });
    healthbar::register(registry);
    dev::register(registry);
}

#[cfg(test)]
mod tests {
    #[tokio::test]
    async fn discover_returns_a_well_formed_list() {
        for service in super::discovery::discover_services().await {
            assert!(service.port > 0 && !service.address.is_empty());
        }
    }
}
