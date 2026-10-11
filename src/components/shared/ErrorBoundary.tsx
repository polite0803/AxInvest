// SPDX-License-Identifier: AGPL-3.0-only

import { Button, Result } from "antd";
import i18next from "i18next";
import React from "react";

interface ErrorBoundaryState {
  hasError: boolean;
  error?: Error;
  /** 抛出点所在的组件链（`in Foo` 逐帧，最内层在前）。dev 下随文案一起上屏。 */
  componentStack?: string | null;
}

interface ErrorBoundaryProps {
  children: React.ReactNode;
  fallback?: React.ReactNode;
  /** 需要拿到 error 才能渲染的兜底（静态 fallback 读不到 state.error）。 */
  renderFallback?: (error: Error | undefined, componentStack: string | null) => React.ReactNode;
  onReset?: () => void;
}

export class ErrorBoundary extends React.Component<
  ErrorBoundaryProps,
  ErrorBoundaryState
> {
  constructor(props: ErrorBoundaryProps) {
    super(props);
    this.state = { hasError: false };
  }

  static getDerivedStateFromError(error: Error): ErrorBoundaryState {
    return { hasError: true, error };
  }

  componentDidCatch(error: Error, errorInfo: React.ErrorInfo) {
    console.error("ErrorBoundary caught an error:", error, errorInfo);
    this.setState({ componentStack: errorInfo.componentStack });
  }

  handleReset = () => {
    this.setState({ hasError: false, error: undefined, componentStack: null });
    this.props.onReset?.();
  };

  render() {
    if (this.state.hasError) {
      if (this.props.fallback) {
        return this.props.fallback;
      }
      if (this.props.renderFallback) {
        return this.props.renderFallback(this.state.error, this.state.componentStack ?? null);
      }
      return (
        <Result
          status="error"
          title={i18next.t("errorBoundary.title")}
          subTitle={this.state.error?.message
            || i18next.t("errorBoundary.unexpectedError")}
          extra={
            <Button type="primary" onClick={this.handleReset}>
              {i18next.t("errorBoundary.tryAgain")}
            </Button>
          }
        />
      );
    }

    return this.props.children;
  }
}

interface PageErrorBoundaryProps {
  children: React.ReactNode;
  title?: string;
}

/**
 * 抛出点定位。优先解析 V8 的 `error.stack`：它自带 `文件:行:列`，且不依赖
 * `componentDidCatch` 里那次额外的 setState 渲染（取不到才退回 `componentStack` 的 `in X` 帧）。
 * 只保留 `.ts`/`.tsx` 帧 ⇒ React 内部帧（`.js`）自然被滤掉。
 */
function topFrames(error: Error | undefined, componentStack: string | null): string {
  const fromStack = (error?.stack ?? "")
    .split("\n")
    .map((l) => l.trim())
    .map((l) => /^at\s+([A-Za-z_$][\w$]*)\s*\(([^()]*?[\\/][\w.-]+\.tsx?:\d+:\d+)\)/.exec(l))
    .filter((m): m is RegExpExecArray => m !== null)
    .map((m) => `${m[1]} @ ${m[2].split(/[\\/]/).pop()}`)
    .slice(0, 3);
  if (fromStack.length > 0) { return fromStack.join(" ← "); }
  return (componentStack ?? "")
    .split("\n")
    .map((l) => l.trim())
    .filter((l) => l.startsWith("in ") && !/^in\s+<Unknown>/.test(l) && !/^in\s+ErrorBoundary\b/.test(l))
    .slice(0, 3)
    .join(" ← ");
}

export function PageErrorBoundary({
  children,
  title = i18next.t("errorBoundary.pageError"),
}: PageErrorBoundaryProps) {
  return (
    <ErrorBoundary
      renderFallback={(error, componentStack) => {
        const frames = topFrames(error, componentStack);
        return (
          <div className="flex items-center justify-center h-full">
            <Result
              status="error"
              title={title}
              // 抛出文本必须上屏：只渲染固定串「页面错误」会让「哪个表达式抛的」
              // 在 UI 与磁盘两侧都取不到读数（前端无 console 落盘桥）。
              subTitle={
                <div
                  className="text-xs"
                  style={{
                    color: "var(--muted)",
                    whiteSpace: "pre-wrap",
                    wordBreak: "break-all",
                    maxWidth: 720,
                    textAlign: "left",
                  }}
                >
                  {i18next.t("errorBoundary.pageError")}
                  {error?.message ? `：${error.message}` : ""}
                  {import.meta.env.DEV && frames
                    ? <div style={{ marginTop: 4, fontFamily: "var(--font-mono)" }}>{frames}</div>
                    : null}
                </div>
              }
              extra={
                <Button type="primary" onClick={() => window.location.reload()}>
                  {i18next.t("errorBoundary.refreshPage")}
                </Button>
              }
            />
          </div>
        );
      }}
    >
      {children}
    </ErrorBoundary>
  );
}
