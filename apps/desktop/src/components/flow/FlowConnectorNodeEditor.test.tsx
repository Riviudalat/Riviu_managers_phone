import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import { useState } from "react";
import { afterEach, expect, it, vi } from "vitest";
import type { ActionKind, JsonObject } from "../../types";
import { FlowConnectorNodeEditor } from "./FlowConnectorNodeEditor";

vi.mock("./FlowConnectorTools", () => ({ FlowConnectorTools: () => <div>Connector settings</div> }));
afterEach(cleanup);
function Fixture({ kind, initial }: {kind: ActionKind; initial: JsonObject}) {
  const [config,setConfig] = useState(initial);
  return <><FlowConnectorNodeEditor kind={kind} config={config} issues={[]} onChange={setConfig} /><output data-testid="value">{JSON.stringify(config)}</output></>;
}
const config = () => JSON.parse(screen.getByTestId("value").textContent!);

it("clears optional HTTP credentials and body and removes body for GET", () => {
  render(<Fixture kind="httpRequest" initial={{ name:"response",url:"https://example.com",method:"POST",body:'{"ok":true}',secretRef:"api",timeoutMs:5000 }} />);
  fireEvent.change(screen.getByLabelText("Tên tham chiếu token (tùy chọn)"), { target:{value:""} });
  expect(config()).not.toHaveProperty("secretRef");
  fireEvent.change(screen.getByLabelText("Nội dung JSON (tùy chọn)"), { target:{value:""} });
  expect(config()).not.toHaveProperty("body");
  fireEvent.change(screen.getByLabelText("Nội dung JSON (tùy chọn)"), { target:{value:"${payload}"} });
  fireEvent.change(screen.getByLabelText("Phương thức HTTP"), { target:{value:"GET"} });
  expect(config()).not.toHaveProperty("body");
  expect(screen.queryByLabelText("Nội dung JSON (tùy chọn)")).toBeNull();
});

it("keeps multiline Sheet data and variable references exactly as authored", () => {
  render(<Fixture kind="sheetWrite" initial={{name:"rows",spreadsheetUrl:"",tab:"",range:"A1:B2",values:""}} />);
  fireEvent.change(screen.getByLabelText("Các hàng cần ghi (JSON)"), {target:{value:'[\n ["a", "b"]\n]'}});
  fireEvent.change(screen.getByLabelText("Link Google Sheet"), {target:{value:"${sheet_url}"}});
  expect(config().values).toBe('[\n ["a", "b"]\n]');
  expect(config().spreadsheetUrl).toBe("${sheet_url}");
  expect(screen.getByText("${rows}")).toBeInTheDocument();
});
