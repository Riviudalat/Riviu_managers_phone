/** Render a tall PNG with a big number centred, for "set number as wallpaper" (A3). */
export async function numberWallpaperPng(label: string): Promise<Uint8Array> {
  const canvas = document.createElement("canvas");
  canvas.width = 1080;
  canvas.height = 1920;
  const ctx = canvas.getContext("2d");
  if (!ctx) throw new Error("no 2d context");
  ctx.fillStyle = "#ffffff";
  ctx.fillRect(0, 0, canvas.width, canvas.height);
  ctx.fillStyle = "#ff6a00";
  ctx.textAlign = "center";
  ctx.textBaseline = "middle";
  const logo = new Image();
  logo.src = new URL(`${import.meta.env.BASE_URL}logo.jpg`, document.baseURI).href;
  await logo.decode();
  const size = 360;
  const scale = Math.min(size / logo.naturalWidth, size / logo.naturalHeight);
  const width = logo.naturalWidth * scale;
  const height = logo.naturalHeight * scale;
  ctx.drawImage(logo, (canvas.width - width) / 2, 440 + (size - height) / 2, width, height);
  ctx.font = `bold ${Math.min(520, 1400 / label.length)}px system-ui, sans-serif`;
  ctx.fillText(label, canvas.width / 2, 1160);
  const blob: Blob = await new Promise((resolve, reject) =>
    canvas.toBlob((b) => (b ? resolve(b) : reject(new Error("toBlob failed"))), "image/png"),
  );
  return new Uint8Array(await blob.arrayBuffer());
}

