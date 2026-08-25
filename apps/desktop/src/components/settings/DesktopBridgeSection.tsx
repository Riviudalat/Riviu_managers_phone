/** Which driver mode the desktop bridge came up in. */
export function DesktopBridgeSection({ mode }: { mode: string }) {

  return (
    <section className="settings-section">
      <h3>Desktop bridge</h3>
      <p className="hint">
        Active mode: <code>{mode}</code>. Mock chỉ dùng khi phát triển.
      </p>
    </section>
  );
}
