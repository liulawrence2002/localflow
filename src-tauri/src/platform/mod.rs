pub trait StartupRegistration: Send + Sync {
    fn set_enabled(&self, enabled: bool) -> Result<(), String>;
    fn is_enabled(&self) -> Result<bool, String>;
}

pub struct NoOpStartupRegistration;

impl StartupRegistration for NoOpStartupRegistration {
    fn set_enabled(&self, _enabled: bool) -> Result<(), String> {
        Ok(())
    }

    fn is_enabled(&self) -> Result<bool, String> {
        Ok(false)
    }
}
