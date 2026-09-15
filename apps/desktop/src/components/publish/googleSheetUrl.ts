export type GoogleSheetTarget = { spreadsheetId: string; sheetId: number; url: string };

export function parseGoogleSheetUrl(raw: string): GoogleSheetTarget | null {
  try {
    const url = new URL(raw.trim());
    const id = /^\/spreadsheets\/d\/([a-zA-Z0-9_-]{1,128})(?:\/edit)?\/?$/.exec(url.pathname)?.[1];
    if (url.protocol !== "https:" || url.hostname !== "docs.google.com" || url.port || url.username || url.password || !id) return null;
    const gids = [...new URLSearchParams(url.hash.slice(1)).getAll("gid"), ...url.searchParams.getAll("gid")];
    const gid = gids[0] ?? "0";
    if (gids.some(value => value !== gid) || !/^\d+$/.test(gid) || !Number.isSafeInteger(Number(gid)) || Number(gid) > 2147483647) return null;
    return { spreadsheetId: id, sheetId: Number(gid), url: `https://docs.google.com/spreadsheets/d/${id}/edit#gid=${Number(gid)}` };
  } catch { return null; }
}
