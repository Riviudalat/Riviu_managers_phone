import { describe, expect, it } from "vitest";
import { GENFARMER_ACTION_INVENTORY, importGenFarmerWorkflow, importMacroWorkflow, type MacroFlowCalibration } from "./flowImport";
import type { Macro } from "./macro";

function source(action = "Pause", options: Record<string, unknown> = { timeoutType: "fixed", timeout: "2.5" }) {
  return { name: "Routine", script: { nodes: [
    { id: "s", data: { action: "Start", options: {}, successNode: "a" } },
    { id: "a", data: { action, options, successNode: "e" } },
    { id: "e", data: { action: "Stop", options: {} } },
  ] } };
}
const convert = (input: unknown) => importGenFarmerWorkflow(JSON.stringify(input));
const codes = (input: ReturnType<typeof convert>) => input.diagnostics.map((item) => item.code);

describe("script import is an explicit graph conversion", () => {
  it("classifies every observed registry action and converts literal assignments/logs",()=>{
    expect(GENFARMER_ACTION_INVENTORY).toHaveLength(62);
    expect(new Set(GENFARMER_ACTION_INVENTORY.map(item=>item.action)).size).toBe(62);
    const assigned=convert(source("SetVariable",{variableName:"message",variableValue:"  hello  ",setOperator:"="}));
    expect(assigned.document?.nodes[1].config).toEqual({name:"message",value:"hello"});
    const logged=convert(source("Log",{log:"hello"}));expect(logged.document?.nodes[1]).toMatchObject({kind:"log",config:{message:"hello"}});
    expect(convert(source("SetVariable",{variableName:"message",variableValue:"${outside}",setOperator:"="})).document).toBeNull();
  });
  it("converts proven string conditions with both branch destinations",()=>{
    const app={name:"condition",script:{nodes:[
      {id:"s",data:{action:"Start",options:{},successNode:"v"}},
      {id:"v",data:{action:"SetVariable",options:{variableName:"x",variableValue:" yes ",setOperator:"="},successNode:"if"}},
      {id:"if",data:{action:"If",options:{leftOperand:"var_x",rightOperand:"yes",operator:"="},successNode:"a",failNode:"b"}},
      {id:"a",data:{action:"Log",options:{log:"yes"},successNode:"end"}},
      {id:"b",data:{action:"Log",options:{log:"no"},successNode:"end"}},
      {id:"end",data:{action:"Stop",options:{}}},
    ]}};
    const imported=convert(app);expect(imported.diagnostics).toEqual([]);
    const predicate=imported.document!.nodes.find(item=>item.kind==="ifValue")!;
    expect(imported.document!.edges.filter(edge=>edge.sourceNodeId===predicate.id).map(edge=>edge.sourcePort)).toEqual(["matched","notMatched"]);
    app.script.nodes[0].data.successNode="if";
    expect(codes(convert(app))).toContain("VariableNotDefinitelyAssigned");
  });
  it("maps bounded Raw GET requests and keeps changed transport policy explicit",()=>{
    const imported=convert(source("HTTP",{url:"https://example.test/status",method:"GET",responseType:"Raw",outputRawVariable:"status",body:"",headers:[],cookies:[],params:[],formData:[]}));
    expect(imported.document?.nodes[1]).toMatchObject({kind:"httpRequest",config:{name:"status",url:"https://example.test/status",method:"GET",timeoutMs:30000},postcondition:{kind:"connectorResult",name:"status"}});
    expect(imported.diagnostics).toContainEqual(expect.objectContaining({code:"HttpExecutionContract",severity:"review"}));
    expect(convert(source("HTTP",{url:"https://example.test",method:"POST",responseType:"Raw",outputRawVariable:"status",headers:[{name:"Authorization",value:"secret"}]})).document).toBeNull();
  });
  it("requires explicit file relocation and preserves line trimming/join semantics",()=>{
    const app=source("ReadFile",{inputType:"file",filePath:"C:\\source.txt",readMode:"lineByLine",outputVariable:"lines",deleteLine:false,randomLine:false});
    expect(codes(convert(app))).toContain("FileDestinationRequired");
    const imported=importGenFarmerWorkflow(JSON.stringify(app),{filePaths:{a:"migrated/source.txt"}});
    expect(imported.document?.nodes.map(n=>n.kind)).toEqual(["start","fileRead","transform","end"]);
    const read=imported.document!.nodes[1];const transform=imported.document!.nodes[2];
    expect(read.config.path).toBe("migrated/source.txt");
    expect(transform.config).toEqual({name:"lines",source:read.config.name,operation:"joinLines",value:","});
    expect(imported.metadata.nodeMap.find(m=>m.sourceNodeId==="a")?.targetNodeIds).toHaveLength(2);
    expect(imported.document?.edges).toContainEqual(expect.objectContaining({sourceNodeId:read.id,targetNodeId:transform.id}));
  });
  it("requires explicit Windows TXT newlines and keeps them in the written value",()=>{
    const app=source("WriteFile",{filePath:"C:\\old.txt",inputVariable:"hello",fileFormat:"txt",writeMode:"overwrite"});
    expect(codes(importGenFarmerWorkflow(JSON.stringify(app),{filePaths:{a:"result.txt"}}))).toContain("SourceNewlineRequired");
    const imported=importGenFarmerWorkflow(JSON.stringify(app),{filePaths:{a:"result.txt"},windowsTextNewlines:true});
    expect(imported.document?.nodes[1]).toMatchObject({kind:"fileWrite",config:{path:"result.txt",format:"text",value:"hello\r\n"}});
  });
  it("converts a bounded stateless LoopV2 with its matching Break into a persisted Repeat body",()=>{
    const app={name:"Loop",script:{nodes:[
      {id:"s",data:{action:"Start",options:{},successNode:"loop"}},
      {id:"loop",data:{action:"LoopV2",options:{loopType:"For",forFrom:"1",forTo:"3",loopId:"L"},successNode:"wait"}},
      {id:"wait",data:{action:"Pause",options:{timeoutType:"fixed",timeout:1},successNode:"break"}},
      {id:"break",data:{action:"Break",options:{loopId:"L",stopLoop:false},successNode:"end"}},
      {id:"end",data:{action:"Stop",options:{}}},
    ]}};
    const result=convert(app);
    expect(result.document?.nodes.map(n=>n.kind)).toEqual(["start","repeat","end"]);
    expect(result.metadata.sourceNodeCount).toBe(5);
    expect(result.metadata.nodeMap.map(item=>item.sourceNodeId)).toEqual(expect.arrayContaining(["s","loop","wait","break","end"]));
    const repeated=result.document!.nodes[1];expect(repeated.config.count).toBe(3);
    expect((repeated.config.document as unknown as {nodes:{kind:string}[]}).nodes.map(n=>n.kind)).toEqual(["start","wait","end"]);
    app.script.nodes[1].data.options.forTo="99";
    expect(codes(convert(app))).toContain("LoopShapeUnsupported");
  });
  it("rejects loop body entry from outside and loopData-dependent effects",()=>{
    const app={name:"Loop",script:{nodes:[
      {id:"s",data:{action:"Start",options:{},successNode:"loop"}},
      {id:"loop",data:{action:"LoopV2",options:{loopType:"For",forFrom:1,forTo:3,loopId:"L"},successNode:"log"}},
      {id:"log",data:{action:"Log",options:{log:"${loopData.L.data}"},successNode:"break"}},
      {id:"break",data:{action:"Break",options:{loopId:"L",stopLoop:false},successNode:"end"}},
      {id:"end",data:{action:"Stop",options:{}}},
    ]}};
    expect(codes(convert(app))).toContain("LoopShapeUnsupported");
  });
  it("reads the observed export envelope and preserves seconds and source edges", () => {
    const app = source();
    // Canvas order does not decide execution order.
    app.script.nodes.reverse();
    const imported = convert({ success: true, data: JSON.stringify(app) });
    expect(imported.diagnostics).toEqual([]);
    expect(imported.document?.revision).toBe(0);
    expect(imported.document?.nodes.find((item) => item.kind === "wait")?.config).toEqual({ durationMs: 2500 });
    const map = new Map(imported.metadata.nodeMap.map((item) => [item.sourceNodeId, item.targetNodeIds[0]]));
    expect(imported.document?.entryNodeId).toBe(map.get("s"));
    expect(imported.document?.edges.map((edge) => [edge.sourceNodeId, edge.targetNodeId])).toEqual([[map.get("a"), map.get("e")], [map.get("s"), map.get("a")]]);
  });

  it("generates deterministic UUID node mappings without mutating source", () => {
    const app = source();
    const before = structuredClone(app);
    const a = convert(app);
    expect(a).toEqual(convert(app));
    expect(app).toEqual(before);
    for (const item of a.document!.nodes) expect(item.id).toMatch(/^[0-9a-f]{8}-[0-9a-f]{4}-4[0-9a-f]{3}-a[0-9a-f]{3}-[0-9a-f]{12}$/);
    expect(new Set(a.document!.nodes.map((item) => item.id)).size).toBe(3);
  });

  it.each(["StartApp", "StopApp"])("adds matching postconditions for %s", (action) => {
    const imported = convert(source(action, { packageName: "com.example.app" }));
    expect(imported.diagnostics).toEqual([]);
    expect(imported.document?.nodes[1].postcondition).toEqual({ kind: action === "StartApp" ? "activeAppEquals" : "processAbsent", bundleId: "com.example.app" });
  });

  it("keeps Home as a draft requiring the actual launcher package", () => {
    const imported = convert(source("PressHome", {}));
    expect(imported.document?.nodes[1]).toMatchObject({ kind: "home", postcondition: null });
    expect(imported.diagnostics[0]).toMatchObject({ severity: "review", sourceNodeId: "a", code: "PostconditionRequired" });
  });

  it.each(["Javascript", "Touch", "Screenshot", "Spreadsheet", "UnknownFutureNode", "toString"])("does not silently drop unsupported %s", (action) => {
    const imported = convert(source(action, {}));
    expect(imported.document).toBeNull();
    expect(imported.diagnostics).toContainEqual(expect.objectContaining({ code: "UnsupportedAction", sourceNodeId: "a", stepIndex: 1 }));
  });

  it("refuses failure branches and nested actions even when their main action converts", () => {
    const app = source();
    Object.assign(app.script.nodes[1].data, { failNode: "e", children: [{ action: "Touch" }] });
    const imported = convert(app);
    expect(imported.document).toBeNull();
    expect(imported.diagnostics.filter((item) => item.code === "BranchUnsupported").map((item) => item.field)).toEqual(["data.failNode", "data.children"]);
    const nested = source();
    Object.assign(nested.script.nodes[1], { parentNode: "external-group" });
    expect(codes(convert(nested))).toContain("NodeStructureUnsupported");
  });

  it.each([
    { timeoutType: "random", timeout: 2, timeoutFrom: 1, timeoutTo: 3 },
    { timeoutType: "fixed", timeout: "${WAIT}" },
    { timeoutType: "fixed", timeout: 61 },
    { timeoutType: "fixed", timeout: 0 },
  ])("rejects timing semantics it cannot preserve", (options) => {
    const imported = convert(source("Pause", options));
    expect(imported.document).toBeNull();
    expect(codes(imported)).toContain("PauseUnsupported");
  });

  it("refuses disabled actions, restart policies, timeouts and data bindings", () => {
    const app = source("StartApp", { packageName: "com.example.app", timeout: 30000, disabled: true, timeoutAdbReconnect: true });
    Object.assign(app.script, { variables: { NAME: "literal" } });
    const imported = convert(app);
    expect(imported.document).toBeNull();
    expect(codes(imported)).toEqual(expect.arrayContaining(["DataBindingUnsupported", "OptionUnsupported", "TimeoutUnsupported"]));
  });

  it("detects cycles, missing destinations, duplicate IDs and unreachable steps", () => {
    const cycle = source();
    cycle.script.nodes[1].data.successNode = "s";
    expect(codes(convert(cycle))).toEqual(expect.arrayContaining(["CycleUnsupported", "UnreachableNode"]));
    const dangling = source();
    dangling.script.nodes[1].data.successNode = "missing";
    expect(codes(convert(dangling))).toContain("MissingEdgeTarget");
    const duplicate = source();
    duplicate.script.nodes[2].id = "a";
    expect(codes(convert(duplicate))).toContain("DuplicateNodeId");
  });

  it("bounds input and rejects malformed envelopes without evaluating strings", () => {
    expect(codes(importGenFarmerWorkflow("x".repeat(1_048_577)))).toContain("FlowImportTooLarge");
    expect(convert({ success: false, data: "{}" }).document).toBeNull();
    expect(convert({ success: true, data: source() }).document).toBeNull();
    expect(importGenFarmerWorkflow("globalThis.sideEffect = true").document).toBeNull();
  });

  it("bounds source arrays and treats prototype-shaped keys as inert JSON data", () => {
    const tooMany = source();
    tooMany.script.nodes = Array.from({ length: 501 }, (_, index) => ({ id: String(index), data: { action: "Start", options: {}, successNode: String(index + 1) } }));
    expect(codes(convert(tooMany))).toContain("NodeCountInvalid");
    const payload = JSON.stringify(source()).replace('"timeoutType":"fixed"', '"__proto__":{"importPolluted":true},"timeoutType":"fixed"');
    expect(codes(importGenFarmerWorkflow(payload))).toContain("OptionUnsupported");
    expect(({} as Record<string, unknown>).importPolluted).toBeUndefined();
  });
});

describe("macro conversion requires facts absent from legacy recordings", () => {
  const macro: Macro = { id: "m", name: "Macro", steps: [{ kind: "tap", x: 100, y: 200, iw: 400, ih: 800, afterMs: 500 }] };
  const measured: MacroFlowCalibration = { imageWidth: 400, imageHeight: 800, orientation: "portrait", profileId: "a".repeat(64), postcondition: { kind: "frameRegionChanged", x: 10, y: 20, width: 100, height: 200, minimumDistance: 1 } };

  it("keeps old coordinates unconverted when orientation and profile are absent", () => {
    const imported = importMacroWorkflow(macro);
    expect(imported.document).toBeNull();
    expect(codes(imported)).toContain("CalibrationRequired");
  });

  it("converts measured coordinates with their evidence and delay in the original order", () => {
    const imported = importMacroWorkflow(macro, { 0: measured });
    expect(imported.document?.nodes.map((item) => item.kind)).toEqual(["start", "tap", "wait", "end"]);
    expect(imported.document?.nodes[1].config.point).toEqual({ x: 100, y: 200, imageWidth: 400, imageHeight: 800, orientation: "portrait", profileId: "a".repeat(64) });
    expect(imported.document?.nodes[1].postcondition).toEqual(measured.postcondition);
    expect(imported.document?.nodes[2].config).toEqual({ durationMs: 500 });
    expect(imported.metadata.nodeMap[0].targetNodeIds).toHaveLength(2);
  });

  it("rejects mismatched dimensions, coordinates and inferred swipe durations", () => {
    expect(codes(importMacroWorkflow(macro, { 0: { ...measured, imageHeight: 801 } }))).toContain("CalibrationRequired");
    const outside = structuredClone(macro);
    Object.assign(outside.steps[0], { x: 400 });
    expect(codes(importMacroWorkflow(outside, { 0: measured }))).toContain("CoordinateOutOfBounds");
    const swipe: Macro = { ...macro, steps: [{ kind: "swipe", x: 100, y: 200, toX: 100, toY: 400, iw: 400, ih: 800, afterMs: 0 }] };
    expect(codes(importMacroWorkflow(swipe, { 0: measured }))).toContain("SwipeEvidenceRequired");
    const imported = importMacroWorkflow(swipe, { 0: { ...measured, swipeDurationMs: 280, postcondition: { kind: "frameDigestChanged", minimumDistance: 1 } } });
    expect(imported.document?.nodes[1].config.durationMs).toBe(280);
  });

  it("converts waits and Home without fabricating launcher evidence", () => {
    const imported = importMacroWorkflow({ ...macro, steps: [{ kind: "wait", afterMs: 500 }, { kind: "key", key: "home", afterMs: 100 }] });
    expect(imported.document?.nodes.map((item) => item.kind)).toEqual(["start", "wait", "home", "wait", "end"]);
    expect(imported.diagnostics[0].severity).toBe("review");
  });

  it("does not silently discard other hardware keys or invalid wait times", () => {
    const imported = importMacroWorkflow({ ...macro, steps: [{ kind: "key", key: "back", afterMs: 100 }] });
    expect(imported.document).toBeNull();
    expect(codes(imported)).toContain("MacroActionUnsupported");
    expect(importMacroWorkflow({ ...macro, steps: [{ kind: "wait", afterMs: -1 }] }).document).toBeNull();
  });

  it("rejects oversized macro arrays, unknown effects and invalid measured regions", () => {
    expect(codes(importMacroWorkflow({ ...macro, steps: Array.from({ length: 501 }, () => ({ kind: "wait" as const, afterMs: 1 })) }))).toContain("MacroShapeInvalid");
    const extra = structuredClone(macro);
    Object.assign(extra.steps[0], { command: "unexpected effect" });
    expect(codes(importMacroWorkflow(extra, { 0: measured }))).toContain("MacroOptionUnsupported");
    expect(codes(importMacroWorkflow(macro, { 0: { ...measured, postcondition: { kind: "frameRegionChanged", x: 390, y: 20, width: 100, height: 10, minimumDistance: 1 } } }))).toContain("PostconditionRequired");
  });
});
