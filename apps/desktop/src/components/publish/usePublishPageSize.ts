import {
  useLayoutEffect,
  useCallback,
  useState,
  type RefCallback,
} from "react";
export function usePublishPageSize(
  rowHeight: number,
  columns = 1,
  maxRows = 20,
): [RefCallback<HTMLDivElement>, number] {
  const [node, setNode] = useState<HTMLDivElement | null>(null);
  const ref = useCallback((value: HTMLDivElement | null) => {
    setNode(value);
  }, []);
  const [size, setSize] = useState(columns * 3);
  useLayoutEffect(() => {
    if (!node) return;
    const update = () => {
      if (node.clientHeight)
        setSize(
          columns *
            Math.max(
              1,
              Math.min(maxRows, Math.floor(node.clientHeight / rowHeight)),
            ),
        );
    };
    update();
    const observer = new ResizeObserver(update);
    observer.observe(node);
    return () => observer.disconnect();
  }, [node, rowHeight, columns, maxRows]);
  return [ref, size];
}
