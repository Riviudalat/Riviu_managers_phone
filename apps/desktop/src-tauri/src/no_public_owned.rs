//! Default-on-drop semantic quarantine; transport teardown is not draft cleanup.
use riviu_core::{DeviceControlPlane, UiWithStreamContext};
use std::sync::Arc;

pub(crate) struct OwnedUi {
    control: Arc<DeviceControlPlane>,
    context: Option<UiWithStreamContext>,
    helper: Option<riviu_android_driver::HelperClient>,
}
impl OwnedUi {
    pub fn new(control: Arc<DeviceControlPlane>, context: UiWithStreamContext) -> Self {
        Self {
            control,
            context: Some(context),
            helper: None,
        }
    }
    pub fn with_helper(mut self, helper: riviu_android_driver::HelperClient) -> Self {
        self.helper = Some(helper);
        self
    }
    pub fn context(&self) -> &UiWithStreamContext {
        self.context
            .as_ref()
            .expect("owned context has not settled")
    }
    pub fn quarantine(mut self) -> anyhow::Result<()> {
        let context = self.context.take().expect("owned context");
        self.control.quarantine_ui_context(context)?;
        Ok(())
    }
    pub async fn close_safe(mut self) -> anyhow::Result<()> {
        // Keep UI ownership through helper settlement/restoration. Failure leaves
        // context armed so Drop quarantines it, rather than releasing for a retry.
        if let Some(helper) = self.helper.take() { helper.shutdown().await?; }
        let context = self.context.take().expect("owned context");
        self.control.close_ui_context(context).await?;
        Ok(())
    }
}
impl Drop for OwnedUi {
    fn drop(&mut self) {
        if let Some(context) = self.context.take() {
            if let Err(error) = self.control.quarantine_ui_context(context) {
                log::error!("diagnostic semantic quarantine refused: {error}");
            }
        }
    }
}
