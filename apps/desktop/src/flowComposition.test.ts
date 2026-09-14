import { describe, expect, it } from "vitest";
import { applyFlowComposition, parseCompositionBindings, parseCompositionBody } from "./flowComposition";
import { newFlowDocument } from "./flow/model";
const ids = () => { let n=0; return () => `id-${++n}`; };

describe("frozen native composition",()=>{
  it("stores a full immutable revision snapshot and explicit mappings",()=>{
    const parent=newFlowDocument("parent"); const body=newFlowDocument("body");body.revision=5;
    const result=applyFlowComposition(parent,{kind:"repeat",document:body,count:3,inputs:{child:"parent"},outputs:{result:"answer"}},null,ids());
    const node=result.nodes.find(n=>n.kind==="repeat")!;
    expect(node.config).toEqual({document:body,count:3,inputs:{child:"parent"},outputs:{result:"answer"}});
    body.name="changed later";
    expect((node.config.document as {name:string}).name).toBe("body");
    expect(parent.nodes).toHaveLength(2);
    expect(result.edges.find(e=>e.targetNodeId===node.id)?.sourceNodeId).toBe(parent.entryNodeId);
  });
  it("updates a selected composition without changing its ID or graph edges",()=>{
    const parent=newFlowDocument();const body=newFlowDocument("body");
    const inserted=applyFlowComposition(parent,{kind:"subflow",document:body,count:1,inputs:{},outputs:{}},null,ids());
    const existing=inserted.nodes.find(n=>n.kind==="subflow")!;
    const updated=applyFlowComposition(inserted,{kind:"repeat",document:body,count:4,inputs:{},outputs:{}},existing.id);
    expect(updated.edges).toEqual(inserted.edges);
    expect(updated.nodes.find(n=>n.id===existing.id)?.config.count).toBe(4);
  });
  it("adds an explicit merge when inserting after converging branches",()=>{
    const parent=newFlowDocument();const body=newFlowDocument();
    const extra={...parent.nodes[0],id:"other",kind:"wait" as const,config:{durationMs:1}};parent.nodes.push(extra);parent.edges.push({...parent.edges[0],id:"other-edge",sourceNodeId:"other"});
    const result=applyFlowComposition(parent,{kind:"subflow",document:body,count:1,inputs:{},outputs:{}},null,ids());
    const join=result.nodes.find(n=>n.kind==="join")!;
    expect(result.edges.filter(e=>e.targetNodeId===join.id)).toHaveLength(2);
    expect(result.edges.filter(e=>e.sourceNodeId===join.id)).toHaveLength(1);
  });
  it("rejects invalid bodies, counts, stale nodes and variable mappings",()=>{
    expect(()=>parseCompositionBody("{}" )).toThrow();
    expect(()=>parseCompositionBindings('{"x":12}')).toThrow();
    expect(()=>parseCompositionBindings('{"bad-name":"x"}')).toThrow();
    expect(parseCompositionBindings('{"child":"parent"}')).toEqual({child:"parent"});
    const doc=newFlowDocument();
    expect(()=>applyFlowComposition(doc,{kind:"repeat",document:doc,count:51,inputs:{},outputs:{}})).toThrow();
    expect(()=>applyFlowComposition(doc,{kind:"subflow",document:doc,count:1,inputs:{},outputs:{}},"missing")).toThrow();
  });
});
