use super::*;
use loopback::{read_request, respond, secure_equal, Request};

const SHEET_MIME: &str = "application/vnd.google-apps.spreadsheet";
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct GooglePickerFile {
    pub id: String,
    pub name: String,
    pub mime_type: String,
}
impl GooglePickerFile {
    pub fn validate(&self) -> Result<()> {
        anyhow::ensure!(
            !self.id.is_empty()
                && self.id.len() <= 256
                && self
                    .id
                    .bytes()
                    .all(|c| c.is_ascii_alphanumeric() || matches!(c, b'_' | b'-')),
            "Google Picker trả ID Sheet không hợp lệ"
        );
        anyhow::ensure!(self.mime_type == SHEET_MIME, "Chọn một Google Sheet");
        anyhow::ensure!(
            !self.name.trim().is_empty()
                && self.name.len() <= 2048
                && !self.name.chars().any(char::is_control),
            "Tên Sheet không hợp lệ"
        );
        Ok(())
    }
}
pub struct GooglePickerSession {
    listener: TcpListener,
    url: String,
    origin: String,
    path: String,
    csrf: String,
    html: String,
    csp: String,
    cancel: GoogleSessionCancel,
    deadline: tokio::time::Instant,
}
impl fmt::Debug for GooglePickerSession {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("GooglePickerSession")
            .field("origin", &self.origin)
            .finish_non_exhaustive()
    }
}
impl GooglePickerSession {
    pub fn picker_url(&self) -> &str {
        &self.url
    }
    pub fn cancel_handle(&self) -> GoogleSessionCancel {
        self.cancel.clone()
    }
    pub async fn wait(self) -> Result<GooglePickerFile> {
        let cancel = self.cancel.clone();
        tokio::select! {biased;_=cancel.cancelled()=>Err(oauth_error(GoogleOAuthErrorCode::Cancelled,"Đã hủy chọn Google Sheet")),result=tokio::time::timeout_at(self.deadline,self.serve())=>result.map_err(|_|oauth_error(GoogleOAuthErrorCode::Timeout,"Hết thời gian chọn Google Sheet"))?}
    }
    async fn serve(self) -> Result<GooglePickerFile> {
        let host = self.listener.local_addr()?.to_string();
        for _ in 0..64 {
            let (mut stream, remote) = self.listener.accept().await?;
            if !remote.ip().is_loopback() {
                continue;
            }
            let request = match read_request(&mut stream, &host).await {
                Ok(r) => r,
                Err(_) => {
                    let _ = respond(&mut stream, 400, "Yêu cầu không hợp lệ.", None).await;
                    continue;
                }
            };
            if request.method == "GET" && request.target == self.path && request.body.is_empty() {
                respond(&mut stream, 200, &self.html, Some(&self.csp)).await?;
                continue;
            }
            if request.target != format!("{}/selected", self.path) {
                let _ = respond(&mut stream, 404, "Không tìm thấy trang.", None).await;
                continue;
            }
            match selected_file(&request, &self.origin, &self.csrf) {
                Ok(Some(file)) => {
                    let _ = respond(
                        &mut stream,
                        200,
                        "Đã chọn Sheet. Trở về Riviu Manager.",
                        None,
                    )
                    .await;
                    return Ok(file);
                }
                Ok(None) => {
                    let _ = respond(&mut stream, 200, "Đã hủy chọn Sheet.", None).await;
                    return Err(oauth_error(
                        GoogleOAuthErrorCode::Cancelled,
                        "Bạn đã hủy chọn Google Sheet",
                    ));
                }
                Err(_) => {
                    let _ =
                        respond(&mut stream, 403, "Kết quả chọn Sheet không hợp lệ.", None).await;
                }
            }
        }
        anyhow::bail!("Quá nhiều yêu cầu không khớp phiên chọn Sheet")
    }
}

pub(super) async fn create_session(
    config: &GoogleOAuthClientConfig,
    tokens: &GoogleOAuthTokens,
) -> Result<GooglePickerSession> {
    config.validate()?;
    let key = config
        .picker_api_key
        .as_deref()
        .context("Thiếu Google Picker API key")?;
    let project = config
        .project_number
        .as_deref()
        .context("Thiếu Google Cloud project number cho Picker")?;
    anyhow::ensure!(
        !tokens.access_token.is_empty()
            && tokens.access_token.len() <= 8192
            && !tokens.needs_refresh(),
        "Làm mới đăng nhập Google trước khi chọn Sheet"
    );
    let listener = TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0)).await?;
    let origin = format!("http://{}", listener.local_addr()?);
    let path = format!("/picker/{}", random_secret());
    let csrf = random_secret();
    let nonce = random_secret();
    let options = serde_json::json!({"token":tokens.access_token,"apiKey":key,"appId":project,"origin":origin,"callback":format!("{path}/selected"),"csrf":csrf});
    let options = script_json(&options);
    let html = format!(
        r#"<!doctype html><html lang="vi"><meta charset="utf-8"><meta name="viewport" content="width=device-width,initial-scale=1"><title>Chọn Google Sheet</title><body><h1>Chọn Google Sheet</h1><p id="status">Đang mở Google Picker…</p><script nonce="{nonce}">
const options={options};let submitted=false;
async function finish(file){{if(submitted)return;submitted=true;try{{const response=await fetch(options.callback,{{method:'POST',headers:{{'Content-Type':'application/json','X-Riviu-Picker-State':options.csrf}},body:JSON.stringify({{file}})}});if(!response.ok)throw new Error('callback');document.getElementById('status').textContent=file?'Đã chọn Sheet. Trở về Riviu Manager.':'Đã hủy chọn Sheet.';}}catch(error){{submitted=false;document.getElementById('status').textContent='Chưa gửi được lựa chọn về ứng dụng. Hãy thử lại.';}}}}
window.pickerLoaded=function(){{gapi.load('picker',{{callback:function(){{const view=new google.picker.DocsView(google.picker.ViewId.SPREADSHEETS).setMimeTypes('{SHEET_MIME}');const picker=new google.picker.PickerBuilder().setDeveloperKey(options.apiKey).setAppId(options.appId).setOAuthToken(options.token).setOrigin(options.origin).addView(view).setCallback(function(data){{if(data.action===google.picker.Action.PICKED){{const doc=data.docs&&data.docs[0];if(doc)finish({{id:doc.id,name:doc.name,mimeType:doc.mimeType}});}}else if(data.action===google.picker.Action.CANCEL)finish(null);}}).build();picker.setVisible(true);document.getElementById('status').textContent='Chọn bảng cần kết nối trong cửa sổ Google.';}},onerror:function(){{document.getElementById('status').textContent='Google Picker chưa tải được. Kiểm tra kết nối và cấu hình Google Cloud.';}}}});}};
</script><script nonce="{nonce}" src="https://apis.google.com/js/api.js?onload=pickerLoaded" async defer></script></body></html>"#
    );
    let csp=format!("default-src 'none'; script-src 'nonce-{nonce}' 'strict-dynamic' https://apis.google.com https://www.gstatic.com; connect-src 'self' https://www.googleapis.com https://content.googleapis.com; frame-src https://docs.google.com https://accounts.google.com; img-src data: https://*.googleusercontent.com https://*.gstatic.com; style-src 'unsafe-inline'; base-uri 'none'; object-src 'none'; frame-ancestors 'none'; form-action 'self'");
    Ok(GooglePickerSession {
        listener,
        url: format!("{origin}{path}"),
        origin,
        path,
        csrf,
        html,
        csp,
        cancel: GoogleSessionCancel::default(),
        deadline: tokio::time::Instant::now() + SESSION_TIMEOUT,
    })
}
fn script_json(value: &serde_json::Value) -> String {
    value
        .to_string()
        .replace('<', "\\u003c")
        .replace('>', "\\u003e")
        .replace('&', "\\u0026")
        .replace('\u{2028}', "\\u2028")
        .replace('\u{2029}', "\\u2029")
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Selection {
    file: Option<GooglePickerFile>,
}
pub(super) fn selected_file(
    request: &Request,
    origin: &str,
    csrf: &str,
) -> Result<Option<GooglePickerFile>> {
    anyhow::ensure!(
        request.method == "POST"
            && request.headers.get("origin").is_some_and(|v| v == origin)
            && request
                .headers
                .get("content-type")
                .is_some_and(|v| v.split(';').next() == Some("application/json"))
            && request
                .headers
                .get("x-riviu-picker-state")
                .is_some_and(|v| secure_equal(v, csrf)),
        "Picker callback origin/state mismatch"
    );
    let selected: Selection =
        serde_json::from_slice(&request.body).context("Invalid Picker selection")?;
    if let Some(file) = &selected.file {
        file.validate()?;
    }
    Ok(selected.file)
}
