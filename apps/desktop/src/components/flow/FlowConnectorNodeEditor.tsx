import type { ActionKind, FlowValidationIssue, JsonObject, JsonValue } from "../../types";
import { flowValidationMessage } from "./validationPresentation";
import { FlowConnectorTools } from "./FlowConnectorTools";

export function FlowConnectorNodeEditor({ kind, config, issues, onChange }: {
  kind: ActionKind; config: JsonObject; issues: FlowValidationIssue[]; onChange: (value: JsonObject) => void;
}) {
  const value = (name: string) => typeof config[name] === "string" ? config[name] as string : "";
  const update = (name: string, next: JsonValue, optional = false) => {
    const changed = { ...config };
    if (optional && next === "") delete changed[name]; else changed[name] = next;
    onChange(changed);
  };
  const field = (name: string, label: string, { multiline = false, optional = false, placeholder = "" } = {}) => <label className="flow-field" key={name}>
    <span>{label}</span>
    {multiline ? <textarea rows={5} value={value(name)} onChange={event => update(name, event.target.value, optional)} placeholder={placeholder} />
      : <input value={value(name)} onChange={event => update(name, event.target.value, optional)} placeholder={placeholder} autoComplete="off" />}
    {issues.filter(issue => issue.field === name || issue.field === `config.${name}`).map((issue,index) => <span role="alert" key={index}>{flowValidationMessage(issue)}</span>)}
  </label>;
  return <>
    {field("name", "Biến lưu kết quả", { placeholder: "result" })}
    <p>Bước sau đọc kết quả bằng <code>{"${" + (value("name") || "result") + "}"}</code>.</p>
    {(kind === "fileRead" || kind === "fileWrite") && <>
      {field("path", "Đường dẫn trong flow-data", { placeholder: "inputs/posts.csv" })}
      <label className="flow-field"><span>Định dạng tệp</span><select value={value("format") || "text"} onChange={event => update("format", event.target.value)}>
        <option value="text">Văn bản UTF-8</option><option value="json">JSON</option><option value="csv">Bảng CSV</option>
      </select></label>
      {kind === "fileWrite" && field("value", "Nội dung cần ghi", { multiline: true, placeholder: "${rows}" })}
    </>}
    {kind === "httpRequest" && <>
      {field("url", "URL yêu cầu", { placeholder: "https://api.example.com/rows" })}
      <label className="flow-field"><span>Phương thức HTTP</span><select value={value("method") || "GET"} onChange={event => {
        const next: JsonObject = { ...config, method: event.target.value }; if (event.target.value === "GET") delete next.body;
        onChange(next);
      }}>{["GET", "POST", "PUT", "PATCH", "DELETE"].map(method => <option key={method}>{method}</option>)}</select></label>
      {value("method") !== "GET" && field("body", "Nội dung JSON (tùy chọn)", { multiline: true, optional: true, placeholder: "${payload}" })}
      {field("secretRef", "Tên tham chiếu token (tùy chọn)", { optional: true, placeholder: "api_quan" })}
      <label className="flow-field"><span>Thời hạn phản hồi (ms)</span><input type="number" min={100} max={30000} step={100}
        value={typeof config.timeoutMs === "number" ? config.timeoutMs : 10000} onChange={event => { const next = event.target.valueAsNumber; if (Number.isFinite(next)) update("timeoutMs", next); }} /></label>
    </>}
    {(kind === "sheetRead" || kind === "sheetWrite") && <>
      {field("spreadsheetUrl", "Link Google Sheet", { placeholder: "${sheet_url}" })}
      {field("tab", "Tên tab chính xác", { placeholder: "Dữ liệu" })}
      {field("range", "Vùng ô A1", { placeholder: "A2:B5" })}
      {kind === "sheetWrite" && field("values", "Các hàng cần ghi (JSON)", { multiline: true, placeholder: '[["Máy 1","Sẵn sàng"]]' })}
      <p>Vùng có tối đa 1.000 ô và nằm trong bảng hiện có. Ghi Sheet yêu cầu số hàng, số cột khớp vùng ô; mọi giá trị là chuỗi.</p>
    </>}
    <FlowConnectorTools />
  </>;
}
