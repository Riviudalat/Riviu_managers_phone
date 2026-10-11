import { GoogleSheetConnection } from "./GoogleSheetConnection";
export function PublishSheetConnection({ network, onReadyChange }: { network?: "threads" | "tiktok"; onReadyChange?: (ready: boolean) => void }) {
  return <GoogleSheetConnection key={network} network={network} onReadyChange={onReadyChange} />;
}
