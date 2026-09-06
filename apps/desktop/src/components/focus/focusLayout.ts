/** Zoom measures the short edge, so rotating a frame does not collapse the controls. */
export function focusLayout(width: number, height: number, zoom: number, viewportWidth: number, viewportHeight: number) {
  const aspect = width > 0 && height > 0 ? height / width : 2;
  const landscape = aspect < 1;
  const stacked = viewportWidth < 700;
  const availableWidth = Math.max(1, viewportWidth - 26 - (stacked ? 0 : 220));
  const wantedWidth = landscape ? zoom / aspect : zoom;
  const availableHeight = Math.max(1, viewportHeight - 26 - (stacked ? 250 : 0));
  const screenWidth = Math.min(wantedWidth, availableWidth, availableHeight / aspect);
  const screenHeight = screenWidth * aspect;
  return { landscape, stacked, screenWidth, screenHeight,
    menuHeight: stacked ? 250 : Math.min(Math.max(screenHeight, 360), viewportHeight - 26) };
}
