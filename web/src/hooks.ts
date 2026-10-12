/** Tiny data-loading hook (no state-management library). */
import { useCallback, useEffect, useRef, useState } from 'react';

export interface AsyncState<T> {
  data: T | null;
  loading: boolean;
  /** second and later loads keep the previous data visible */
  refreshing: boolean;
  error: unknown;
  reload: () => void;
  setData: (updater: T | null | ((previous: T | null) => T | null)) => void;
}

/**
 * Runs `loader` whenever `deps` change. The loader receives an `AbortSignal` that is
 * aborted on unmount/re-run, so pages can simply pass it to the api client.
 */
export function useAsync<T>(loader: (signal: AbortSignal) => Promise<T>, deps: unknown[]): AsyncState<T> {
  const [data, setDataState] = useState<T | null>(null);
  const [loading, setLoading] = useState(true);
  const [refreshing, setRefreshing] = useState(false);
  const [error, setError] = useState<unknown>(null);
  const [tick, setTick] = useState(0);
  const hasLoaded = useRef(false);

  const loaderRef = useRef(loader);
  loaderRef.current = loader;

  useEffect(() => {
    const controller = new AbortController();
    let active = true;
    if (hasLoaded.current) setRefreshing(true);
    else setLoading(true);
    setError(null);

    loaderRef
      .current(controller.signal)
      .then((result) => {
        if (!active) return;
        setDataState(result);
        hasLoaded.current = true;
      })
      .catch((cause: unknown) => {
        if (!active) return;
        if (cause instanceof DOMException && cause.name === 'AbortError') return;
        setError(cause);
      })
      .finally(() => {
        if (!active) return;
        setLoading(false);
        setRefreshing(false);
      });

    return () => {
      active = false;
      controller.abort();
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [...deps, tick]);

  const reload = useCallback(() => setTick((value) => value + 1), []);

  const setData = useCallback((updater: T | null | ((previous: T | null) => T | null)) => {
    setDataState((previous) => (typeof updater === 'function' ? (updater as (p: T | null) => T | null)(previous) : updater));
  }, []);

  return { data, loading, refreshing, error, reload, setData };
}

/** Debounces a fast-changing value (search boxes). */
export function useDebounced<T>(value: T, delayMs = 300): T {
  const [debounced, setDebounced] = useState(value);
  useEffect(() => {
    const timer = window.setTimeout(() => setDebounced(value), delayMs);
    return () => window.clearTimeout(timer);
  }, [value, delayMs]);
  return debounced;
}
