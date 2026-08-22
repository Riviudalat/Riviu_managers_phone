import { useEffect, useRef } from "react";
import { attachViewCanvas, detachViewCanvas } from "../viewStore";

interface Props {
  udid: string;
  surfaceId: string;
  className?: string;
  fill?: boolean;
}

function canvasClassName(fill: boolean | undefined, className: string | undefined): string {
  return `phone-canvas${fill ? " is-fill" : ""}${className ? ` ${className}` : ""}`;
}

/**
 * A fresh <canvas> is created for every effect run. `transferControlToOffscreen`
 * can only be called once per element, and React StrictMode remounts effects
 * on the same fiber — reusing the JSX canvas throws InvalidStateError and
 * unmounts the whole tile tree.
 *
 * **Which is why `className` must not be a dependency of that effect.** It was, and the
 * overlay passes `` `focus-touch${busy ? " is-busy" : ""}` `` — so every menu action that
 * flipped `busy` tore the canvas down and built another one. Tearing it down detaches the
 * surface, which closes the decoder and drops the worker's slot, so the picture went black
 * and stayed black until the next keyframe: a visible flicker on every rotate, install, adb
 * call, import, export and screenshot, twice each (on and off again).
 *
 * The class is a property of an element that already exists, so it is set on the element
 * rather than rebuilt with it.
 */
export function PhoneCanvas({ udid, surfaceId, className, fill }: Props) {
  const hostRef = useRef<HTMLDivElement>(null);
  const canvasRef = useRef<HTMLCanvasElement | null>(null);
  // The current appearance, readable from the identity effect without becoming one of its
  // dependencies. Keeping `className` out of those deps is what stopped the canvas being
  // rebuilt on every `busy` flip; reading it here is what stops the rebuilt canvas — the
  // one a change of `udid` legitimately creates — from coming up with no class at all.
  const appearanceRef = useRef({ fill, className });
  appearanceRef.current = { fill, className };

  useEffect(() => {
    const host = hostRef.current;
    if (!host) return;
    const canvas = document.createElement("canvas");
    canvas.dataset.udid = udid;
    // Styled before it is in the document, so there is never a frame of unstyled canvas
    // and never a canvas that stays unstyled: the effect below only re-runs when the
    // appearance itself changes, and switching phones changes neither.
    canvas.className = canvasClassName(
      appearanceRef.current.fill,
      appearanceRef.current.className,
    );
    host.appendChild(canvas);
    canvasRef.current = canvas;
    const forget = () => {
      canvas.remove();
      if (canvasRef.current === canvas) canvasRef.current = null;
    };
    if (typeof canvas.transferControlToOffscreen !== "function") {
      return forget;
    }
    const offscreen = canvas.transferControlToOffscreen();
    attachViewCanvas(udid, offscreen, surfaceId);
    return () => {
      detachViewCanvas(udid, surfaceId);
      forget();
    };
    // Identity only. A canvas belongs to one device and one surface; everything else about
    // it is appearance, and appearance is not a reason to throw a decoder away.
  }, [surfaceId, udid]);

  // Runs on mount too, so the canvas is never briefly unstyled.
  useEffect(() => {
    if (canvasRef.current) canvasRef.current.className = canvasClassName(fill, className);
  }, [className, fill]);

  return <div ref={hostRef} className="phone-canvas-host" data-udid={udid} />;
}
