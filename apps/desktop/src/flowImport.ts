import type { Macro } from "./macro";
import type { ActionKind, EvidenceSpec, FlowDocumentV2, FlowNode, JsonObject, ScreenOrientation } from "./types";

export const MAX_FLOW_IMPORT_BYTES = 1_048_576;
const MAX_SOURCE_NODES = 500;

/** Export registry observed in the source, including actions whose contracts still need adapters. */
export const GENFARMER_ACTION_INVENTORY = ["Adb", "BackupRestoreV2", "Break", "CasePath", "ChangeDevice", "CheckActivity", "CheckNetwork", "CheckPlatformAccount", "ClearAppData", "Clipboard", "Cmd", "DeepSeek", "DeviceAction", "Dialog", "ElementExists", "Gemini", "GenRouter", "GetAttributeValue", "GetProperty", "Grok", "GroupNode", "HTTP", "If", "Image", "Imap", "InsertData", "InstallApp", "IsInstalledApp", "Javascript", "Log", "Loop", "LoopV2", "MultiElementExists", "OpenAi", "OtherScript", "Pause", "Press", "PressBack", "PressHome", "PressMenu", "Random", "ReadFile", "Reconnect", "RegEx", "SaveAssets", "Screenshot", "SelectDropdown", "SetVariable", "Spreadsheet", "StartApp", "Stop", "StopApp", "Swipe", "ToggleService", "Touch", "TransferFile", "TwoFA", "TypeText", "UninstallApp", "UpdateField", "WriteFile", "Xpath"].map((action) => ({ action, status: ["Pause", "PressHome", "StartApp", "Stop", "StopApp", "SetVariable", "Log", "If", "HTTP", "ReadFile", "WriteFile", "LoopV2", "Break"].includes(action) ? "supportedSubset" as const : "adapterRequired" as const, reason: importActionReason(action) }));

function importActionReason(action:string):string {
  if(["LoopV2","Break"].includes(action))return "For cố định, body tuyến tính không phụ thuộc loopData và Break cùng loopId.";
  if(action==="HTTP")return "GET Raw cố định; hiển thị giới hạn timeout/redirect trước khi nhận bản nháp.";
  if(action==="ReadFile")return "Đọc và biến đổi dòng; chọn đường dẫn mới trong flow-data trước khi nhập tệp.";
  if(action==="WriteFile")return "TXT ghi đè; chọn đích flow-data và xác nhận CRLF của nguồn.";
  if(action==="If")return "Chuỗi đã gán trên mọi nhánh; =, !=, hasValue, giữ hai nhánh và điểm gộp.";
  if(action==="SetVariable")return "Gán = chuỗi cố định; bảo toàn trim của nguồn.";
  if(action==="Log")return "Thông điệp cố định; không tự biến biểu thức thành mã chạy.";
  if(["Pause","PressHome","StartApp","Stop","StopApp"].includes(action))return "Kiểm tra từng tùy chọn; Home cần launcher thật và app cần hậu điều kiện.";
  if(action==="Spreadsheet")return "Nguồn cloud ghi append USER_ENTERED; node Riviu ghi đúng vùng và kiểm tra giá trị. Cần chọn lại hành vi đọc/ghi.";
  if(action==="Screenshot")return "Đích file, crop và biến ảnh nguồn khác artifact chụp ảnh Riviu; cần ánh xạ đầu ra.";
  if(["Touch","Swipe","TypeText","ElementExists","MultiElementExists","GetAttributeValue","SelectDropdown","Xpath","Image","CasePath"].includes(action))return "Cần chuyển selector/tọa độ, nhiều kết quả và điều kiện xác minh sang hợp đồng UI Riviu.";
  if(["OpenAi","Gemini","Grok","DeepSeek","Imap","TwoFA","CheckPlatformAccount","GenRouter"].includes(action))return "Cần provider, tài khoản và ánh xạ phản hồi của dịch vụ tương ứng.";
  if(["InstallApp","UninstallApp","ClearAppData","DeviceAction","ToggleService","TransferFile","Clipboard","Press","PressBack","PressMenu","GetProperty","CheckActivity","CheckNetwork","Reconnect","BackupRestoreV2","ChangeDevice"].includes(action))return "Cần adapter thiết bị, kiểm tra khả năng và hậu điều kiện cho thao tác nguồn.";
  if(["OtherScript","GroupNode","Loop"].includes(action))return "Cần snapshot và đường chạy con hoàn chỉnh, ánh xạ biến/điểm thoát; không suy từ tên node.";
  if(["Adb","Cmd","Javascript"].includes(action))return "Nội dung lệnh/script cần được dựng bằng hành động có hợp đồng thực thi riêng.";
  return "Cần ánh xạ kiểu dữ liệu, đầu vào/đầu ra và tác dụng phụ của hành động nguồn trước khi chuyển.";
}

export interface FlowImportDiagnostic {
  severity: "error" | "review";
  sourceNodeId: string | null;
  stepIndex: number | null;
  code: string;
  field: string | null;
  message: string;
}

export interface WorkflowImportResult {
  document: FlowDocumentV2 | null;
  diagnostics: FlowImportDiagnostic[];
  metadata: {
    format: "genfarmer" | "macro";
    sourceName: string;
    sourceNodeCount: number;
    nodeMap: { sourceNodeId: string; targetNodeIds: string[] }[];
    draftOnly: true;
  };
}

/** Explicit measurements supplied with a recording. Old macros do not carry these facts. */
export interface MacroFlowCalibration {
  imageWidth: number;
  imageHeight: number;
  orientation: ScreenOrientation;
  profileId: string;
  postcondition: EvidenceSpec;
  swipeDurationMs?: number;
}

export interface WorkflowImportOptions {
  /** Explicit migration destination under Riviu flow-data, keyed by source node ID. */
  filePaths?: Record<string, string>;
  windowsTextNewlines?: boolean;
}

type RecordValue = Record<string, unknown>;
const record = (value: unknown): value is RecordValue => typeof value === "object" && value !== null && !Array.isArray(value);
const empty = (value: unknown): boolean => value === undefined || value === null || value === "";
const inert = (value: unknown): boolean => empty(value) || value === false || (record(value) && Object.keys(value).length === 0) || (Array.isArray(value) && value.length === 0);
const boundedInteger = (value: unknown, min: number, max: number): value is number => typeof value === "number" && Number.isSafeInteger(value) && value >= min && value <= max;

export function parseWorkflowImport(raw: string): unknown {
  if (new TextEncoder().encode(raw).byteLength > MAX_FLOW_IMPORT_BYTES) throw new Error("FlowImportTooLarge");
  return JSON.parse(raw) as unknown;
}

// Stable UUID-shaped identities for a draft, not an integrity/security hash. The workspace
// assigns a new flow ID when accepting an import; node/edge mappings remain reproducible.
function stableId(seed: string): string {
  const words = [0x811c9dc5, 0x9e3779b9, 0x85ebca6b, 0xc2b2ae35].map((initial) => {
    let hash = initial;
    for (let i = 0; i < seed.length; i++) hash = Math.imul(hash ^ seed.charCodeAt(i), 0x01000193);
    return (hash >>> 0).toString(16).padStart(8, "0");
  }).join("");
  return `${words.slice(0, 8)}-${words.slice(8, 12)}-4${words.slice(13, 16)}-a${words.slice(17, 20)}-${words.slice(20)}`;
}

function createResult(format: "genfarmer" | "macro", sourceName: string): WorkflowImportResult {
  return { document: null, diagnostics: [], metadata: { format, sourceName, sourceNodeCount: 0, nodeMap: [], draftOnly: true } };
}

function issue(result: WorkflowImportResult, code: string, message: string, stepIndex: number | null = null, sourceNodeId: string | null = null, field: string | null = null, severity: "error" | "review" = "error") {
  result.diagnostics.push({ severity, code, message, stepIndex, sourceNodeId, field });
}

function node(seed: string, sourceId: string, index: number, kind: ActionKind, config: JsonObject = {}, postcondition: EvidenceSpec | null = null): FlowNode {
  return { id: stableId(`${seed}/node/${sourceId}`), kind, config, postcondition, position: { x: index * 240, y: 80 } };
}

function finish(result: WorkflowImportResult, seed: string, nodes: FlowNode[], connections: [string, string, string?][]) {
  if (result.diagnostics.some((item) => item.severity === "error")) return result;
  result.document = {
    schemaVersion: 2, id: stableId(`${seed}/flow`), name: result.metadata.sourceName, revision: 0,
    entryNodeId: nodes.find((item) => item.kind === "start")!.id, nodes,
    edges: connections.map(([sourceNodeId, targetNodeId, port]) => ({ id: stableId(`${seed}/edge/${sourceNodeId}/${port ?? "flow"}/${targetNodeId}`), sourceNodeId, sourcePort: port ?? "flow", targetNodeId, targetPort: "flow" })),
    viewport: { x: 0, y: 0, zoom: 1 },
  };
  return result;
}

/** Import only observed export shapes. No eval, device calls, filesystem effects or IPC. */
export function importGenFarmerWorkflow(raw: string, importOptions: WorkflowImportOptions = {}): WorkflowImportResult {
  return convertGenFarmerWorkflow(raw,importOptions,0);
}

function convertGenFarmerWorkflow(raw: string, importOptions: WorkflowImportOptions, depth:number): WorkflowImportResult {
  const result = createResult("genfarmer", "Flow nhập từ script");
  let app: unknown;
  try {
    app = parseWorkflowImport(raw);
    if (record(app) && "success" in app) {
      if (app.success !== true || typeof app.data !== "string") throw new Error("ExportEnvelopeInvalid");
      app = parseWorkflowImport(app.data);
    }
  } catch (error) {
    issue(result, error instanceof Error ? error.message : "InvalidJson", "Tệp phải là JSON export hợp lệ, tối đa 1 MiB.");
    return result;
  }
  if (!record(app) || !record(app.script) || !Array.isArray(app.script.nodes)) {
    issue(result, "ScriptShapeUnsupported", "Cần đối tượng app có script.nodes hoặc envelope export {success, data}.");
    return result;
  }
  const script = app.script;
  if (typeof app.name === "string" && app.name.trim()) result.metadata.sourceName = app.name.trim().slice(0, 200);
  let sourceNodes = script.nodes as unknown[];
  result.metadata.sourceNodeCount = sourceNodes.length;
  if(depth>8){issue(result,"CompositionDepthLimit","Kịch bản lồng quá 8 cấp.");return result;}
  if (sourceNodes.length < 2 || sourceNodes.length > MAX_SOURCE_NODES) {
    issue(result, "NodeCountInvalid", "Script phải có từ 2 đến 500 node.");
    return result;
  }
  for (const field of ["variables", "input", "table"]) {
    if (!inert(script[field]) || !inert(app[field])) issue(result, "DataBindingUnsupported", "Dữ liệu đầu vào hoặc biến cần được chuyển đổi trước khi nhập.", null, null, field);
  }
  if (!inert(script.options) && (!record(script.options) || Object.entries(script.options).some(([, value]) => !inert(value)))) issue(result, "ScriptOptionsUnsupported", "Tùy chọn chạy cần được chuyển đổi tường minh.", null, null, "script.options");
  const seed = `genfarmer/${stableId(raw)}`;
  const composed=new Map<string,FlowDocumentV2>();
  const originalIndices=new Map(sourceNodes.filter(record).filter(item=>typeof item.id==="string").map((item,index)=>[item.id as string,index]));
  for(const source of [...sourceNodes]){
    if(!record(source)||typeof source.id!=="string"||!record(source.data)||source.data.action!=="LoopV2"||!record(source.data.options))continue;
    const loop=source.data.options;
    const fail=(message:string)=>issue(result,"LoopShapeUnsupported",message,originalIndices.get(source.id as string)??null,source.id as string,"options");
    const number=(value:unknown)=>typeof value==="number"?value:typeof value==="string"&&/^-?\d+$/.test(value)?Number(value):NaN;
    const from=number(loop.forFrom),to=number(loop.forTo),count=to-from+1;
    if(loop.loopType!=="For"||!Number.isSafeInteger(from)||!Number.isSafeInteger(to)||count<1||count>50||typeof loop.loopId!=="string"||!loop.loopId){fail("Chuyển được LoopV2 For từ/to cố định, tối đa 50 lượt và có loopId.");continue;}
    if(Object.entries(loop).some(([field,value])=>!["loopType","forFrom","forTo","loopId"].includes(field)&&!inert(value))){fail("Loop có tùy chọn bổ sung chưa có chuyển đổi tương đương.");continue;}
    const byId=new Map(sourceNodes.filter(record).filter(item=>typeof item.id==="string").map(item=>[item.id,item]));
    const body:RecordValue[]=[];const seen=new Set<string>();let cursor=source.data.successNode;let stop:RecordValue|undefined;
    while(typeof cursor==="string"&&!seen.has(cursor)){
      seen.add(cursor);const item=byId.get(cursor);if(!item||!record(item.data)||!record(item.data.options))break;
      if(item.data.action==="Break"&&item.data.options.loopId===loop.loopId){stop=item;break;}
      // A stateless linear loop is exact without exposing the source loopData counter.
      if(!["Pause","PressHome","StartApp","StopApp","Log"].includes(String(item.data.action))||!inert(item.data.failNode)||JSON.stringify(item.data.options).includes("${"))break;
      body.push(item);cursor=item.data.successNode;
    }
    if(!stop||!record(stop.data)||!record(stop.data.options)||body.length===0||!inert(stop.data.options.stopLoop)||typeof stop.data.successNode!=="string"||seen.has(stop.data.successNode)||stop.data.successNode===source.id){fail("Loop cần body tuyến tính không dùng loopData, đóng bằng Break cùng loopId và nối ra bước ngoài vòng.");continue;}
    const bodyIds=new Set(body.map(item=>item.id));
    const outsideIncoming=sourceNodes.some(item=>record(item)&&record(item.data)&&item.id!==source.id&&!bodyIds.has(item.id)&&[item.data.successNode,item.data.failNode].some(target=>bodyIds.has(target)||target===stop!.id));
    if(outsideIncoming){fail("Có cạnh ngoài đi vào body/Break; cần tách vòng lặp trước khi nhập.");continue;}
    if(Object.entries(stop.data.options).some(([key,value])=>!["loopId","stopLoop"].includes(key)&&!inert(value))||Object.entries(stop.data).some(([key,value])=>!["action","options","successNode","name","label","description"].includes(key)&&!inert(value))||Object.entries(stop).some(([key,value])=>!["id","data","type","position","positionAbsolute","width","height","selected","dragging"].includes(key)&&!inert(value))){fail("Break có nhánh hoặc tùy chọn bổ sung chưa tương thích.");continue;}
    const startId=`${source.id}/body-start`,endId=`${source.id}/body-end`;
    const bodyNodes=body.map(item=>({ ...structuredClone(item),data:{...(item.data as RecordValue),successNode:(item.data as RecordValue).successNode===stop!.id?endId:(item.data as RecordValue).successNode} }));
    const imported=convertGenFarmerWorkflow(JSON.stringify({name:`${result.metadata.sourceName} · ${source.id}`,script:{nodes:[{id:startId,data:{action:"Start",options:{},successNode:body[0].id}},...bodyNodes,{id:endId,data:{action:"Stop",options:{}}}]}}),importOptions,depth+1);
    if(!imported.document){for(const diagnostic of imported.diagnostics)result.diagnostics.push(diagnostic);continue;}
    result.diagnostics.push(...imported.diagnostics);
    composed.set(source.id,imported.document);
    result.metadata.nodeMap.push(...imported.metadata.nodeMap.filter(mapping=>mapping.sourceNodeId!==startId&&mapping.sourceNodeId!==endId));
    result.metadata.nodeMap.push({sourceNodeId:stop.id as string,targetNodeIds:[imported.document.nodes.find(item=>item.kind==="end")!.id]});
    source.data={...source.data,successNode:stop.data.successNode};
    sourceNodes=sourceNodes.filter(item=>!record(item)||(!bodyIds.has(item.id)&&item.id!==stop!.id));
    issue(result,"LoopExpanded","LoopV2 và Break được chuyển thành Lặp Flow con với số lượt cố định.",originalIndices.get(source.id)??null,source.id,"options","review");
  }
  const ids = new Map<string, number>();
  const next = new Map<string, { target: string; port: string }[]>();
  const nodes: FlowNode[] = [];
  const starts: string[] = [];
  const stops: string[] = [];
  const chains = new Map<string,FlowNode[]>();
  sourceNodes.forEach((source, index) => {
    if (!record(source) || typeof source.id !== "string" || !source.id || !record(source.data) || typeof source.data.action !== "string" || !record(source.data.options)) {
      issue(result, "NodeShapeInvalid", "Node cần id, data.action và data.options.", index);
      return;
    }
    const id = source.id;
    if (ids.has(id)) issue(result, "DuplicateNodeId", "ID node bị trùng.", index, id, "id");
    ids.set(id, index);
    const { data } = source;
    const action = data.action as string;
    const options = data.options as RecordValue;
    const error = (code: string, message: string, field: string | null = null) => issue(result, code, message, index, id, field);
    const review = (message: string) => issue(result, "PostconditionRequired", message, index, id, "postcondition", "review");
    for (const [field, value] of Object.entries(source)) {
      if (!["id", "data", "type", "position", "positionAbsolute", "width", "height", "selected", "dragging"].includes(field) && !inert(value)) error("NodeStructureUnsupported", "Node có cấu trúc bổ sung cần được chuyển đổi tường minh.", field);
    }
    for (const [field, value] of Object.entries(data)) {
      if (!["action", "options", "successNode", "name", "label", "description", ...(action === "If" ? ["failNode"] : [])].includes(field) && !inert(value)) error("BranchUnsupported", "Nhánh hoặc dữ liệu node chưa có chuyển đổi tương đương.", `data.${field}`);
    }
    const allowed: Record<string, string[]> = { Start: [], Stop: [], PressHome: [], Pause: ["timeoutType", "timeout", "timeoutFrom", "timeoutTo"], StartApp: ["packageName", "timeout"], StopApp: ["packageName"], SetVariable: ["variableName", "variableValue", "setOperator"], Log: ["log"], If: ["leftOperand", "rightOperand", "operator"], HTTP:["url","method","responseType","outputRawVariable","contentType","headers","cookies","params","body","formData","outputBodyVariables"],ReadFile:["inputType","inputVariable","filePath","deleteLine","randomLine","readMode","lineDelimiter","mapVariables","outputVariable"],WriteFile:["filePath","inputVariable","fileFormat","writeMode","appendMode","appendDelimiter","csvDelimiter"],...(composed.has(id)?{LoopV2:["loopType","forFrom","forTo","loopId"]}:{}) };
    if (!Object.hasOwn(allowed, action)) {
      error("UnsupportedAction", `${action}: ${importActionReason(action)}`, "data.action");
      return;
    }
    for (const [field, value] of Object.entries(options)) {
      if (!allowed[action].includes(field) && !inert(value)) error("OptionUnsupported", "Tùy chọn này chưa được chuyển đổi; bản nhập sẽ không bỏ tùy chọn.", `options.${field}`);
    }
    let converted: FlowNode;
    const after: FlowNode[] = [];
    if(action==="LoopV2"&&composed.has(id)){
      converted=node(seed,id,index,"repeat",{document:composed.get(id)! as unknown as JsonObject,count:Number(options.forTo)-Number(options.forFrom)+1,inputs:{},outputs:{}});
    } else if (action === "Start") {
      starts.push(id);
      converted = node(seed, id, index, "start");
    } else if (action === "Stop") {
      stops.push(id);
      converted = node(seed, id, index, "end");
      if (!empty(data.successNode)) error("StopHasOutgoingEdge", "Node kết thúc vẫn có nhánh đi ra.", "successNode");
    } else if (action === "Pause") {
      const seconds = typeof options.timeout === "string" && /^\d+(\.\d+)?$/.test(options.timeout) ? Number(options.timeout) : options.timeout;
      const durationMs = typeof seconds === "number" ? seconds * 1000 : NaN;
      if (options.timeoutType !== "fixed" || !boundedInteger(durationMs, 1, 60_000)) error("PauseUnsupported", "Chuyển được thời gian chờ cố định từ 0,001 đến 60 giây; ngẫu nhiên cần node riêng.", "options.timeout");
      converted = node(seed, id, index, "wait", { durationMs });
    } else if (action === "PressHome") {
      converted = node(seed, id, index, "home");
      review("Chọn package màn hình Home làm hậu điều kiện trước khi lưu/chạy.");
    } else if (action === "SetVariable") {
      const name = options.variableName;
      const value = options.variableValue;
      if (options.setOperator !== "=" || typeof name !== "string" || !/^[A-Za-z_][A-Za-z0-9_]{0,63}$/.test(name) || typeof value !== "string" || value.includes("${") || value.startsWith("var_") || value.length > 4096) error("VariableAssignmentUnsupported", "Chuyển được phép gán = chuỗi cố định; phép tính, đối tượng hoặc nội suy cần chuyển đổi riêng.", "options");
      converted = node(seed, id, index, "setVariable", { name: typeof name === "string" ? name : "", value: typeof value === "string" ? value.trim() : "" });
    } else if (action === "Log") {
      const message = options.log;
      if (typeof message !== "string" || !message || message.length > 4096 || message.includes("${") || message.startsWith("var_")) error("LogUnsupported", "Chuyển được thông điệp nhật ký cố định; nội suy cần ánh xạ biến riêng.", "options.log");
      converted = node(seed, id, index, "log", { message: typeof message === "string" ? message : "" });
    } else if (action === "HTTP") {
      const name=options.outputRawVariable;
      const url=options.url;
      let validUrl=false;
      try {const parsed=new URL(String(url));validUrl=(parsed.protocol==="https:"||(parsed.protocol==="http:"&&["127.0.0.1","localhost","[::1]"].includes(parsed.hostname)))&&!parsed.username&&!parsed.password&&!parsed.hash;}catch{/* diagnosed below */}
      if(options.responseType!=="Raw"||typeof name!=="string"||!/^[A-Za-z_][A-Za-z0-9_]{0,63}$/.test(name)||!validUrl||typeof url!=="string"||url.includes("${")||url.length>2048||options.method!=="GET"||!inert(options.body)||!inert(options.headers)||!inert(options.cookies)||!inert(options.params)||!inert(options.outputBodyVariables)||!inert(options.formData)) error("HttpOptionsUnsupported","Chuyển được GET URL cố định trả Raw, chưa có header/cookie/body/mapping; cấu hình khác cần adapter HTTP riêng.","options");
      converted=node(seed,id,index,"httpRequest",{name:typeof name==="string"?name:"",url:typeof url==="string"?url:"",method:"GET",timeoutMs:30000},{kind:"connectorResult",name:typeof name==="string"?name:""});
      issue(result,"HttpExecutionContract","HTTP dùng giới hạn 30 giây, không theo redirect và lưu phản hồi tối đa 4.096 ký tự. Kiểm tra endpoint trước khi chạy.",index,id,"options","review");
    } else if(action==="ReadFile") {
      const name=options.outputVariable;
      const sourceName=options.inputVariable;
      const output=typeof name==="string"?name:"";
      if(!/^[A-Za-z_][A-Za-z0-9_]{0,63}$/.test(output)||!inert(options.deleteLine)||!inert(options.randomLine)||!inert(options.mapVariables)||!["lineByLine","lineByLineDelimiter"].includes(String(options.readMode))||(options.readMode==="lineByLineDelimiter"&&!empty(options.lineDelimiter))) error("ReadFileOptionsUnsupported","Chuyển đọc toàn bộ dòng nối dấu phẩy hoặc dòng đầu không delimiter; xóa/ngẫu nhiên/base64 cần xử lý riêng.","options");
      const input=options.inputType==="file"?`importFile_${stableId(id).replaceAll("-","")}`:typeof sourceName==="string"?sourceName:"";
      const transform={name:output,source:input,operation:options.readMode==="lineByLine"?"joinLines":"firstLine",...(options.readMode==="lineByLine"?{value:","}:{})};
      if(options.inputType==="file"){
        const path=importOptions.filePaths?.[id];
        if(typeof path!=="string"||!path||path.startsWith("/")||path.includes(":")||path.includes("\\")||path.split("/").some(part=>!part||part==="."||part===".."))error("FileDestinationRequired","Chọn đường dẫn tương đối trong flow-data cho tệp nguồn rồi nhập lại.","filePaths");
        converted=node(seed,id,index,"fileRead",{name:input,path:path??"",format:"text"});
        after.push(node(seed,`${id}/lines`,index+1,"transform",transform));
        issue(result,"FileMigrationRequired","Đặt tệp nguồn vào flow-data theo đường dẫn đã chọn trước khi chạy.",index,id,"filePaths","review");
      }else{
        if(typeof sourceName!=="string"||!/^[A-Za-z_][A-Za-z0-9_]{0,63}$/.test(sourceName))error("ReadFileInputInvalid","Biến nguồn phải có tên hợp lệ.","inputVariable");
        converted=node(seed,id,index,"transform",transform);
      }
    } else if(action==="WriteFile"){
      const path=importOptions.filePaths?.[id];const value=options.inputVariable;
      const name=`importWrite_${stableId(id).replaceAll("-","")}`;
      if(options.fileFormat!=="txt"||options.writeMode!=="overwrite"||typeof value!=="string"||value.length>4094||value.startsWith("var_"))error("WriteFileOptionsUnsupported","Chuyển được ghi đè tệp TXT; append, CSV và JSON cần adapter giữ đúng định dạng.","options");
      if(!importOptions.windowsTextNewlines)error("SourceNewlineRequired","Xác nhận kịch bản TXT nguồn dùng xuống dòng Windows trước khi chuyển.","windowsTextNewlines");
      if(typeof path!=="string"||!path||path.startsWith("/")||path.includes(":")||path.includes("\\")||path.split("/").some(part=>!part||part==="."||part===".."))error("FileDestinationRequired","Chọn đường dẫn tương đối trong flow-data cho tệp kết quả.","filePaths");
      converted=node(seed,id,index,"fileWrite",{name,path:path??"",format:"text",value:typeof value==="string"?`${value}\r\n`:""},{kind:"connectorResult",name});
      issue(result,"FileMigrationRequired","Tệp kết quả được ghi tại flow-data theo đường dẫn đã chọn và đọc lại để kiểm chứng.",index,id,"filePaths","review");
    } else if (action === "If") {
      const operand = typeof options.leftOperand === "string" ? options.leftOperand.replace(/^var_/, "") : "";
      const writers = sourceNodes.filter((item) => record(item) && record(item.data) && record(item.data.options) && item.data.options.variableName === operand);
      const literalWriter = writers.length > 0 && writers.every((item) => { const data = (item as { data: { action: string; options: RecordValue } }).data; return data.action === "SetVariable" && data.options.setOperator === "=" && typeof data.options.variableValue === "string" && !data.options.variableValue.includes("${"); });
      const operators: Record<string, string> = { "=": "equals", "!=": "notEquals", hasValue: "notEmpty" };
      const right = options.rightOperand;
      if (!literalWriter || !/^[A-Za-z_][A-Za-z0-9_]{0,63}$/.test(operand) || typeof right !== "string" || right.includes("${") || typeof options.operator !== "string" || !Object.hasOwn(operators, options.operator)) error("ComparisonUnsupported", "Chuyển được =, != hoặc hasValue trên biến chuỗi đã gán cố định trong script; so sánh số và biến ngoài cần chuẩn hóa riêng.", "options");
      converted = node(seed, id, index, "ifValue", { name: operand, operator: operators[String(options.operator)] ?? "equals", value: typeof right === "string" ? right.trim() : "" });
    } else {
      const bundleId = options.packageName;
      if (typeof bundleId !== "string" || !/^[A-Za-z0-9_]+(?:\.[A-Za-z0-9_]+)+$/.test(bundleId) || bundleId.length > 255) error("PackageInvalid", "Package phải là giá trị cố định hợp lệ; biến nội suy cần chuyển đổi riêng.", "options.packageName");
      if (action === "StartApp" && !empty(options.timeout) && Number(options.timeout) !== 10_000) error("TimeoutUnsupported", "Flow hiện dùng giới hạn mở app 10000 ms; script có giới hạn khác cần điều chỉnh tường minh.", "options.timeout");
      const target = typeof bundleId === "string" ? bundleId : "";
      converted = node(seed, id, index, action === "StartApp" ? "launchApp" : "terminateApp", { bundleId: target }, { kind: action === "StartApp" ? "activeAppEquals" : "processAbsent", bundleId: target });
    }
    nodes.push(converted,...after);
    chains.set(id,[converted,...after]);
    result.metadata.nodeMap.push({ sourceNodeId: id, targetNodeIds: [converted.id,...after.map(item=>item.id)] });
    if (action !== "Stop") {
      if (typeof data.successNode !== "string" || !data.successNode) error("MissingSuccessEdge", "Node chưa nối tới bước tiếp theo.", "successNode");
      else next.set(id, [{ target: data.successNode, port: action === "If" ? "matched" : "flow" }]);
      if (action === "If") {
        if (typeof data.failNode !== "string" || !data.failNode) error("MissingFailureEdge", "Điều kiện cần nối nhánh không khớp.", "failNode");
        else next.set(id, [...(next.get(id) ?? []), { target: data.failNode, port: "notMatched" }]);
      }
    }
  });
  if (starts.length !== 1 || stops.length !== 1) issue(result, "EntryExitInvalid", "Bộ chuyển đổi yêu cầu đúng một Start và một Stop.");
  for (const [id, targets] of next) for (const { target } of targets) if (!ids.has(target)) issue(result, "MissingEdgeTarget", "Nhánh nối tới ID node không tồn tại.", ids.get(id) ?? null, id, "successNode");
  const visited = new Set<string>();
  const active = new Set<string>();
  const visit = (id: string) => {
    if (active.has(id)) { issue(result, "CycleUnsupported", "Script có vòng lặp; cần chuyển sang vòng lặp có giới hạn trước khi nhập.", ids.get(id) ?? null, id); return; }
    if (visited.has(id)) return;
    visited.add(id); active.add(id);
    for (const target of next.get(id) ?? []) visit(target.target);
    active.delete(id);
  };
  if (starts[0]) visit(starts[0]);
  for (const source of sourceNodes) {
    if (!record(source) || !record(source.data) || source.data.action !== "If" || !record(source.data.options) || typeof source.id !== "string") continue;
    const name = String(source.data.options.leftOperand ?? "").replace(/^var_/, "");
    const memo=new Map<string,boolean>();
    const definite = (id: string, path: Set<string>): boolean => {
      if (path.has(id)) return false;
      if(memo.has(id))return memo.get(id)!;
      const item = sourceNodes[ids.get(id) ?? -1];
      if (record(item) && record(item.data) && item.data.action === "SetVariable" && record(item.data.options) && item.data.options.variableName === name) return true;
      if (starts.includes(id)) return false;
      const previous = [...next].filter(([, targets]) => targets.some((target) => target.target === id)).map(([from]) => from);
      const assigned=previous.length > 0 && previous.every((from) => definite(from, new Set([...path,id])));
      memo.set(id,assigned);return assigned;
    };
    if (!definite(source.id, new Set())) issue(result,"VariableNotDefinitelyAssigned","Biến điều kiện cần được gán trên mọi nhánh trước khi so sánh.",ids.get(source.id) ?? null,source.id,"leftOperand");
  }
  for (const [id, index] of ids) if (!visited.has(id)) issue(result, "UnreachableNode", "Node không nằm trên đường chạy; hãy nối hoặc xử lý node này trước khi nhập.", index, id);
  const mapped = new Map(result.metadata.nodeMap.map((item) => [item.sourceNodeId, item.targetNodeIds[0]]));
  const connections: [string,string,string?][] = [...next].flatMap(([from, targets]) => targets.map(({target,port}):[string,string,string] => [chains.get(from)?.at(-1)?.id??mapped.get(from)!, mapped.get(target)!, port]));
  for(const chain of chains.values())for(let i=1;i<chain.length;i++)connections.push([chain[i-1].id,chain[i].id,"flow"]);
  // Branch rejoin needs an explicit native Join before ordinary action nodes.
  for (const target of [...nodes]) {
    const incoming = connections.filter((edge) => edge[1] === target.id);
    if (incoming.length > 1 && target.kind !== "end") {
      const join = node(seed, `join/${target.id}`, nodes.length, "join");
      nodes.push(join);
      for (const edge of incoming) edge[1] = join.id;
      connections.push([join.id,target.id,"flow"]);
    }
  }
  return finish(result, seed, nodes, connections);
}

export function importMacroWorkflow(macro: Macro, calibration: Record<string, MacroFlowCalibration> = {}): WorkflowImportResult {
  const result = createResult("macro", typeof macro?.name === "string" && macro.name.trim() ? macro.name.trim().slice(0, 200) : "Flow từ Macro");
  if (!record(macro) || !Array.isArray(macro.steps) || macro.steps.length === 0 || macro.steps.length > MAX_SOURCE_NODES) {
    issue(result, "MacroShapeInvalid", "Macro cần từ 1 đến 500 bước.");
    return result;
  }
  result.metadata.sourceNodeCount = macro.steps.length;
  const seed = `macro/${stableId(`${JSON.stringify(macro)}/${JSON.stringify(calibration)}`)}`;
  const nodes: FlowNode[] = [node(seed, "start", 0, "start")];
  macro.steps.forEach((step, index) => {
    const id = String(index);
    const error = (code: string, message: string, field: string | null = null) => issue(result, code, message, index, id, field);
    if (!record(step) || !boundedInteger(step.afterMs, 0, 60_000)) { error("MacroStepInvalid", "Bước cần thời gian chờ nguyên từ 0 đến 60000 ms.", "afterMs"); return; }
    const fields = step.kind === "tap" ? ["kind", "x", "y", "iw", "ih", "afterMs"] : step.kind === "swipe" ? ["kind", "x", "y", "toX", "toY", "iw", "ih", "afterMs"] : step.kind === "key" ? ["kind", "key", "afterMs"] : ["kind", "afterMs"];
    for (const field of Object.keys(step)) if (!fields.includes(field)) error("MacroOptionUnsupported", "Trường bổ sung chưa có chuyển đổi tương đương.", field);
    const targetIds: string[] = [];
    const append = (suffix: string, kind: ActionKind, config: JsonObject = {}, evidence: EvidenceSpec | null = null) => {
      const item = node(seed, `${id}/${suffix}`, nodes.length, kind, config, evidence);
      nodes.push(item); targetIds.push(item.id);
    };
    if (step.kind === "key" && step.key === "home") {
      append("home", "home");
      issue(result, "PostconditionRequired", "Chọn package màn hình Home làm hậu điều kiện trước khi lưu/chạy.", index, id, "postcondition", "review");
    } else if (step.kind === "tap" || step.kind === "swipe") {
      const frame = calibration[id];
      if (!frame || !boundedInteger(frame.imageWidth, 1, 32768) || !boundedInteger(frame.imageHeight, 1, 32768) || frame.imageWidth !== step.iw || frame.imageHeight !== step.ih || !/^[0-9a-f]{64}$/.test(frame.profileId) || !["portrait", "portraitUpsideDown", "landscapeLeft", "landscapeRight"].includes(frame.orientation)) {
        error("CalibrationRequired", "Bản ghi thiếu profile và hướng khung hình gốc khớp iw/ih. Cần hiệu chỉnh trước khi chuyển tọa độ.", "calibration");
      } else {
        const point = (x: number, y: number) => ({ x, y, imageWidth: frame.imageWidth, imageHeight: frame.imageHeight, orientation: frame.orientation, profileId: frame.profileId });
        const inFrame = (x: number, y: number) => Number.isFinite(x) && Number.isFinite(y) && x >= 0 && y >= 0 && x < frame.imageWidth && y < frame.imageHeight;
        if (!inFrame(step.x, step.y) || (step.kind === "swipe" && !inFrame(step.toX, step.toY))) error("CoordinateOutOfBounds", "Tọa độ nằm ngoài khung hình gốc.");
        else if (step.kind === "tap") {
          if (frame.postcondition?.kind !== "frameRegionChanged" || !boundedInteger(frame.postcondition.x, 0, frame.imageWidth - 1) || !boundedInteger(frame.postcondition.y, 0, frame.imageHeight - 1) || !boundedInteger(frame.postcondition.width, 1, frame.imageWidth - frame.postcondition.x) || !boundedInteger(frame.postcondition.height, 1, frame.imageHeight - frame.postcondition.y) || !boundedInteger(frame.postcondition.minimumDistance, 1, 255)) error("PostconditionRequired", "Chạm cần vùng thay đổi hợp lệ nằm trong khung hình đã đo.", "postcondition");
          else append("tap", "tap", { point: point(step.x, step.y) }, frame.postcondition);
        } else if (!boundedInteger(frame.swipeDurationMs, 1, 5000) || frame.postcondition?.kind !== "frameDigestChanged" || !boundedInteger(frame.postcondition.minimumDistance, 1, 255)) error("SwipeEvidenceRequired", "Vuốt cần thời lượng gốc và hậu điều kiện thay đổi khung hình.", "calibration");
        else append("swipe", "swipe", { from: point(step.x, step.y), to: point(step.toX, step.toY), durationMs: frame.swipeDurationMs }, frame.postcondition);
      }
    } else if (step.kind !== "wait") error("MacroActionUnsupported", "Phím hoặc thao tác này chưa có node Flow tương đương.", "kind");
    if (step.afterMs > 0) append("wait", "wait", { durationMs: step.afterMs });
    if (step.kind === "wait" && step.afterMs === 0) error("EmptyWait", "Bước chờ 0 ms cần được chỉnh lại trước khi nhập.", "afterMs");
    result.metadata.nodeMap.push({ sourceNodeId: id, targetNodeIds: targetIds });
  });
  nodes.push(node(seed, "end", nodes.length, "end"));
  if (nodes.length > MAX_SOURCE_NODES) issue(result, "ConvertedNodeLimit", "Macro tạo hơn 500 node sau khi tách thời gian chờ; hãy chia bản ghi.");
  return finish(result, seed, nodes, nodes.slice(1).map((item, index) => [nodes[index].id, item.id]));
}
