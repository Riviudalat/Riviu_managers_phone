import { useEffect, useRef } from "react";
import { attachViewCanvas, detachViewCanvas } from "../viewStore";

interface Props {
  udid: string;
  surfaceId: string;
  className?: string;
  fill?: boolean;
}

/**
 * A fresh <canvas> is created for every effect run. `transferControlToOffscreen`
 * can only be called once per element, and React StrictMode remounts effects
 * on the same fiber — reusing the JSX canvas throws InvalidStateError and
 * unmounts the whole tile tree.
 */
export function PhoneCanvas({ udid, surfaceId, className, fill }: Props) {
  const hostRef = useRef<HTMLDivElement>(null);

  useEffect(() => {
    const host = hostRef.current;
    if (!host) return;
    const canvas = document.createElement("canvas");
    canvas.className = `phone-canvas${fill ? " is-fill" : ""}${className ? ` ${className}` : ""}`;
    canvas.dataset.udid = udid;
    host.appendChild(canvas);
    if (typeof canvas.transferControlToOffscreen !== "function") {
      return () => canvas.remove();
    }
    const offscreen = canvas.transferControlToOffscreen();
    attachViewCanvas(udid, offscreen, surfaceId);
    return () => {
      detachViewCanvas(udid, surfaceId);
      canvas.remove();
    };
  }, [className, fill, surfaceId, udid]);

  return <div ref={hostRef} className="phone-canvas-host" data-udid={udid} />;
}
