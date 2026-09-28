import { GoogleSheetConnection, type SheetReadyChange } from "./GoogleSheetConnection";
export function PublishSheetConnection({ onReadyChange }: { onReadyChange?: SheetReadyChange }) {
  return <GoogleSheetConnection onReadyChange={onReadyChange} />;
}
