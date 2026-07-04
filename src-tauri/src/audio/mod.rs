#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AudioDevice {
    pub id: String,
    pub name: String,
}

pub trait AudioCapture: Send + Sync {
    fn list_devices(&self) -> Result<Vec<AudioDevice>, String>;
    fn start(&self, device_id: &str) -> Result<(), String>;
    fn stop(&self) -> Result<(), String>;
}

#[derive(Default)]
pub struct MockAudioCapture;

impl AudioCapture for MockAudioCapture {
    fn list_devices(&self) -> Result<Vec<AudioDevice>, String> {
        Ok(vec![AudioDevice {
            id: "default".to_string(),
            name: "Default microphone".to_string(),
        }])
    }

    fn start(&self, _device_id: &str) -> Result<(), String> {
        Ok(())
    }

    fn stop(&self) -> Result<(), String> {
        Ok(())
    }
}
