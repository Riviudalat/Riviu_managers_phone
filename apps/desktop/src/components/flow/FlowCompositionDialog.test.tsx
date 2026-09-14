import { cleanup,fireEvent,render,screen,waitFor } from "@testing-library/react";
import { afterEach,describe,expect,it,vi } from "vitest";
import { newFlowDocument } from "../../flow/model";
import { FlowCompositionDialog } from "./FlowCompositionDialog";
const api=vi.hoisted(()=>({flowGet:vi.fn(),flowValidate:vi.fn(async()=>({}))}));
vi.mock("../../api",()=>api);
afterEach(()=>{cleanup();vi.clearAllMocks();});

describe("composition dialog",()=>{
  it("loads the chosen saved revision and validates the combined parent before applying",async()=>{
    const doc=newFlowDocument("parent"); const child=newFlowDocument("child");child.revision=4;
    api.flowGet.mockResolvedValue({document:child});
    const apply=vi.fn();render(<FlowCompositionDialog document={doc} flows={[{id:child.id,name:"child",latestRevision:4,archived:false,updatedAt:"now"}]} editingNode={null} onApply={apply} onClose={vi.fn()}/>);
    fireEvent.change(screen.getByLabelText("Flow đã lưu"),{target:{value:child.id}});
    fireEvent.click(screen.getByRole("button",{name:"Nạp đúng phiên bản"}));
    await waitFor(()=>expect(screen.getByLabelText("JSON Flow con")).toHaveValue(JSON.stringify(child,null,2)));
    expect(api.flowGet).toHaveBeenCalledWith(child.id,4);
    fireEvent.change(screen.getByLabelText("Cách thực hiện"),{target:{value:"repeat"}});
    fireEvent.change(screen.getByLabelText("Số lượt Flow con"),{target:{value:"3"}});
    fireEvent.change(screen.getByLabelText("Biến đầu vào (biến con: biến cha)"),{target:{value:'{"child":"parent"}'}});
    fireEvent.click(screen.getByRole("button",{name:"Áp dụng Flow con"}));
    await waitFor(()=>expect(apply).toHaveBeenCalledTimes(1));
    const node=apply.mock.calls[0][0].nodes.find((item:{kind:string})=>item.kind==="repeat");
    expect(node.config).toEqual({document:child,count:3,inputs:{child:"parent"},outputs:{}});
    expect(api.flowValidate).toHaveBeenCalledWith(apply.mock.calls[0][0]);
  });
  it("keeps validation errors visible and does not apply a half valid composition",async()=>{
    api.flowValidate.mockRejectedValueOnce([{code:"MissingVariable",message:"missing input"}]);
    const apply=vi.fn();render(<FlowCompositionDialog document={newFlowDocument()} flows={[]} editingNode={null} onApply={apply} onClose={vi.fn()}/>);
    fireEvent.click(screen.getByRole("button",{name:"Áp dụng Flow con"}));
    expect(await screen.findByRole("alert")).toHaveTextContent("missing input");expect(apply).not.toHaveBeenCalled();
  });
  it("ignores a stale file/revision response after the operator edits the body",async()=>{
    const child=newFlowDocument("child");child.revision=2;
    let release:(value:unknown)=>void=()=>{};api.flowGet.mockImplementationOnce(()=>new Promise(resolve=>{release=resolve;}));
    render(<FlowCompositionDialog document={newFlowDocument()} flows={[{id:child.id,name:"child",latestRevision:2,archived:false,updatedAt:"now"}]} editingNode={null} onApply={vi.fn()} onClose={vi.fn()}/>);
    fireEvent.change(screen.getByLabelText("Flow đã lưu"),{target:{value:child.id}});fireEvent.click(screen.getByRole("button",{name:"Nạp đúng phiên bản"}));
    fireEvent.change(screen.getByLabelText("JSON Flow con"),{target:{value:"edited"}});release({document:child});
    await waitFor(()=>expect(screen.getByLabelText("JSON Flow con")).toHaveValue("edited"));
  });
});
