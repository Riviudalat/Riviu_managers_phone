import { GoogleSheetConnection, type SheetReadyChange } from "./GoogleSheetConnection";
export function PublishSheetConnection({ onReadyChange, onLinkChange }: { onReadyChange?: SheetReadyChange; onLinkChange?: (link: string) => void }) {
  return <GoogleSheetConnection onReadyChange={onReadyChange} onLinkChange={onLinkChange} />;
}
