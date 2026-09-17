import { useCallback, useEffect, useRef, useState } from "react";
import { describeError } from "./describeError";

/** Nguồn đọc cần ổn định; đổi nguồn sẽ bỏ dữ liệu của phạm vi trước. */
export function useAsyncList<T>(read: () => Promise<T>) {
  const activeSource = useRef<(() => Promise<T>) | null>(null);
  const generation = useRef(0);
  const [state, setState] = useState<{
    source: () => Promise<T>;
    data: T | undefined;
    error: string | null;
    loading: boolean;
  }>({ source: read, data: undefined, error: null, loading: true });

  const load = useCallback(async () => {
    // Callback giữ bởi thao tác cũ không được đọc lại sau đổi phạm vi/unmount.
    if (activeSource.current !== read) return;
    const request = ++generation.current;
    const isCurrent = () => activeSource.current === read && generation.current === request;
    setState((current) => ({
      source: read,
      data: current.source === read ? current.data : undefined,
      error: null,
      loading: true,
    }));
    try {
      const data = await read();
      if (isCurrent()) setState({ source: read, data, error: null, loading: true });
    } catch (cause) {
      if (isCurrent()) setState((current) => ({ ...current, error: describeError(cause) }));
    } finally {
      if (isCurrent()) setState((current) => ({ ...current, loading: false }));
    }
  }, [read]);

  useEffect(() => {
    activeSource.current = read;
    void load();
    return () => {
      // Vô hiệu ngay; lần setup kế tiếp tăng generation trong load trước await.
      activeSource.current = null;
    };
  }, [read, load]);

  // Không trình bày hàng của kind cũ ngay cả ở render trước effect mới.
  const sameSource = state.source === read;
  const data = sameSource ? state.data : undefined;
  const loading = !sameSource || state.loading;
  return {
    data,
    error: sameSource ? state.error : null,
    loading,
    initialLoading: loading && data === undefined,
    refreshing: loading && data !== undefined,
    load,
  };
}
