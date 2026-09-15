import { GoogleSheetConnection } from "./GoogleSheetConnection";
export function PublishSheetConnection({ onReadyChange }: { onReadyChange?: (ready: boolean) => void }) {
  return <GoogleSheetConnection onReadyChange={onReadyChange} />;
}
