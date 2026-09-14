use super::*;
use crate::flow::connectors::FlowConnectorExecutor;

impl FlowExecutor {
    pub(super) fn values_at(
        &self,
        attempt_id: Uuid,
    ) -> Result<std::collections::BTreeMap<String, String>, FlowExecutionError> {
        let context = self
            .deps
            .database
            .get_flow_attempt_execution_context(attempt_id)
            .map_err(FlowExecutionError::other)?
            .ok_or_else(|| {
                FlowExecutionError::new("RunIdentityMismatch", "data consumer is missing")
            })?;
        super::super::data::variables_before(
            &context.plan,
            &context.device_attempts,
            context.device.id,
            context.attempt.node_id,
        )
        .map_err(|e| FlowExecutionError::new("VariableStateInvalid", e))
    }

    pub(super) async fn dispatch_connector(
        &self,
        node: &CompiledFlowNode,
        attempt_id: Uuid,
    ) -> Result<ActionOutput, ActionDispatchFailure> {
        let values = self
            .values_at(attempt_id)
            .map_err(ActionDispatchFailure::deterministic_read)?;
        let resolve = |s: &str| {
            super::super::extended::resolve_text(s, &values)
                .map_err(|e| FlowExecutionError::new("VariableReferenceInvalid", e))
        };
        let service = FlowConnectorExecutor::new(
            self.deps.database.flow_connector_root(),
            self.deps.database.clone(),
        )
        .map_err(|e| {
            ActionDispatchFailure::non_delivery(FlowExecutionError::new(e.code, e.message))
        })?;
        use CompiledActionConfig as C;
        let result = match &node.config {
            C::FileRead(c) => {
                let mut c = c.clone();
                c.path = resolve(&c.path).map_err(ActionDispatchFailure::non_delivery)?;
                service.file_read(&c).await
            }
            C::FileWrite(c) => {
                let mut c = c.clone();
                c.path = resolve(&c.path).map_err(ActionDispatchFailure::non_delivery)?;
                c.value = resolve(&c.value).map_err(ActionDispatchFailure::non_delivery)?;
                service.file_write(&c).await
            }
            C::HttpRequest(c) => {
                let mut c = c.clone();
                c.url = resolve(&c.url).map_err(ActionDispatchFailure::non_delivery)?;
                c.body = c
                    .body
                    .as_deref()
                    .map(resolve)
                    .transpose()
                    .map_err(ActionDispatchFailure::non_delivery)?;
                service.http_request(&c).await
            }
            C::SheetRead(c) => {
                let mut c = c.clone();
                c.spreadsheet_url =
                    resolve(&c.spreadsheet_url).map_err(ActionDispatchFailure::non_delivery)?;
                c.tab = resolve(&c.tab).map_err(ActionDispatchFailure::non_delivery)?;
                c.range = resolve(&c.range).map_err(ActionDispatchFailure::non_delivery)?;
                service.sheet_read(&c).await
            }
            C::SheetWrite(c) => {
                let mut c = c.clone();
                c.spreadsheet_url =
                    resolve(&c.spreadsheet_url).map_err(ActionDispatchFailure::non_delivery)?;
                c.tab = resolve(&c.tab).map_err(ActionDispatchFailure::non_delivery)?;
                c.range = resolve(&c.range).map_err(ActionDispatchFailure::non_delivery)?;
                c.values = resolve(&c.values).map_err(ActionDispatchFailure::non_delivery)?;
                service.sheet_write(&c).await
            }
            _ => {
                return Err(FlowExecutionError::new(
                    "CompiledPlanCorrupt",
                    "connector config mismatch",
                )
                .into())
            }
        };
        result.map(ActionOutput::Data).map_err(|e| {
            let err = FlowExecutionError::new(e.code, e.message);
            if e.may_have_applied {
                err.into()
            } else {
                ActionDispatchFailure::non_delivery(err)
            }
        })
    }

    pub(super) async fn dispatch_ocr(
        &self,
        node: &CompiledFlowNode,
        context: &FlowDeviceContext,
        attempt_id: Uuid,
    ) -> Result<ActionOutput, ActionDispatchFailure> {
        let CompiledActionConfig::OcrReadText {
            name,
            region,
            languages,
            min_confidence,
        } = &node.config
        else {
            return Err(
                FlowExecutionError::new("CompiledPlanCorrupt", "OCR config mismatch").into(),
            );
        };
        let session = context
            .session(&self.deps.control)
            .map_err(FlowExecutionError::device)?;
        let reasoner = self
            .deps
            .reasoner
            .clone()
            .or_else(|| session.gui_reasoner())
            .ok_or_else(|| {
                FlowExecutionError::new(
                    "CapabilityUnavailable",
                    "local OCR service is unavailable in this session",
                )
            })?;
        let generation = context.generation();
        let frame = self
            .deps
            .frames
            .latest_in_generation(&self.deps.udid, generation)
            .ok_or_else(|| {
                FlowExecutionError::new("StaleGeneration", "OCR frame is unavailable")
            })?;
        let decoded =
            decode_and_hash_artifact(&frame.bytes).map_err(FlowExecutionError::evidence)?;
        use base64::Engine as _;
        let roi = region.map(|r| {
            let x = (r.x0 * f64::from(decoded.width)).floor() as u32;
            let y = (r.y0 * f64::from(decoded.height)).floor() as u32;
            crate::ui_automation::OcrRect {
                x,
                y,
                width: (r.x1 * f64::from(decoded.width)).ceil() as u32 - x,
                height: (r.y1 * f64::from(decoded.height)).ceil() as u32 - y,
            }
        });
        let request = crate::ui_automation::OcrRequest {
            protocol_version: 1,
            request_id: attempt_id.to_string(),
            observation_id: format!("{}:{}", generation, frame.sequence),
            session_epoch: self.deps.run_id.to_string(),
            generation,
            remaining_ms: 30_000,
            screenshot: crate::ui_automation::OcrImage {
                bytes_base64: base64::engine::general_purpose::STANDARD.encode(&*frame.bytes),
                sha256: decoded.sha256,
                width: decoded.width,
                height: decoded.height,
            },
            roi,
            languages: languages.clone(),
            min_confidence: *min_confidence,
        };
        request.validate().map_err(FlowExecutionError::other)?;
        let response = reasoner
            .ocr(request.clone())
            .await
            .map_err(|e| ActionDispatchFailure::deterministic_read(FlowExecutionError::other(e)))?;
        response
            .validate_binding(&request)
            .map_err(|e| ActionDispatchFailure::deterministic_read(FlowExecutionError::other(e)))?;
        if self
            .deps
            .frames
            .latest_in_generation(&self.deps.udid, generation)
            .is_none()
        {
            return Err(ActionDispatchFailure::deterministic_read(
                FlowExecutionError::new(
                    "StaleGeneration",
                    "OCR observation belongs to an old session",
                ),
            ));
        }
        Ok(ActionOutput::Data(
            serde_json::json!({"kind":"flowVariable","name":name,"value":response.text,"ocr":response}),
        ))
    }
}
