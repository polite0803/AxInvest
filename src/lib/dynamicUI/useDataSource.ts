// SPDX-License-Identifier: AGPL-3.0-only

import type { DataSourceConfig, SkillPermissions } from "@/types";
import { useEffect, useRef, useState } from "react";
import { resolveDataSource, subscribeDataSource } from "./DataBindingEngine";

interface UseDataSourceResult<T = unknown> {
  data: T | undefined;
  loading: boolean;
  error: Error | null;
  refresh: () => Promise<void>;
}

/**
 * `permissions` 必须是**所在渲染树**的来源权限面（见 `useSchemaActionPermissions`）。
 * 不传即视为不受限 ⇒ 只能用于可信来源的树；插件来源的树漏传就是开绕过口。
 */
export function useDataSource<T = unknown>(
  config: DataSourceConfig | undefined,
  permissions?: SkillPermissions,
): UseDataSourceResult<T> {
  const [data, setData] = useState<T | undefined>(undefined);
  const [loading, setLoading] = useState<boolean>(!!config);
  const [error, setError] = useState<Error | null>(null);
  const subscriberRef = useRef<{ unsubscribe: () => void } | null>(null);

  const refresh = async () => {
    if (!config) {
      setData(undefined);
      setLoading(false);
      setError(null);
      return;
    }
    setLoading(true);
    setError(null);
    try {
      const result = await resolveDataSource(config, permissions);
      setData(result as T);
    } catch (err) {
      setError(err instanceof Error ? err : new Error(String(err)));
    } finally {
      setLoading(false);
    }
  };

  useEffect(() => {
    let mounted = true;

    if (!config) {
      setTimeout(() => {
        setData(undefined);
        setLoading(false);
        setError(null);
      }, 0);
      return;
    }

    setTimeout(() => {
      setLoading(true);
      setError(null);
    }, 0);

    void (async () => {
      try {
        if (subscriberRef.current) {
          subscriberRef.current.unsubscribe();
        }

        subscriberRef.current = await subscribeDataSource(
          config,
          (newData) => {
            if (mounted) {
              setData(newData as T);
              setLoading(false);
              setError(null);
            }
          },
          (err) => {
            if (mounted) {
              setError(err);
              setLoading(false);
            }
          },
          permissions,
        );
      } catch (err) {
        if (mounted) {
          setError(err instanceof Error ? err : new Error(String(err)));
          setLoading(false);
        }
      }
    })();

    return () => {
      mounted = false;
      if (subscriberRef.current) {
        subscriberRef.current.unsubscribe();
        subscriberRef.current = null;
      }
    };
  }, [config, permissions, setData]);

  return { data, loading, error, refresh };
}
