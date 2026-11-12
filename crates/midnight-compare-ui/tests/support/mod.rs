use slint::platform::{
    Platform, PlatformError, WindowAdapter, software_renderer::MinimalSoftwareWindow,
};
use std::rc::Rc;

pub struct TestPlatform(pub Rc<MinimalSoftwareWindow>);
impl Platform for TestPlatform {
    fn create_window_adapter(&self) -> Result<Rc<dyn WindowAdapter>, PlatformError> {
        Ok(self.0.clone())
    }
}
