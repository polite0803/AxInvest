import type { ArtifactLanguage } from "@/types";
import type { editor as monacoEditor } from "monaco-editor";
import { useEffect, useRef } from "react";

// WikiEditorPage 等文件仍通过 window.monaco 访问，保留全局类型声明
declare global {
  interface Window {
    monaco: typeof import("monaco-editor");
  }
}

const LANGUAGE_MAP: Record<ArtifactLanguage, string> = {
  javascript: "javascript",
  typescript: "typescript",
  jsx: "javascript",
  tsx: "typescript",
  html: "html",
  css: "css",
  python: "python",
  markdown: "markdown",
  text: "plaintext",
  json: "json",
  svg: "xml",
  mermaid: "markdown",
  d2: "markdown",
};

interface MonacoEditorProps {
  value: string;
  language: ArtifactLanguage;
  onChange?: (value: string) => void;
  readOnly?: boolean;
  height?: string | number;
}

export function MonacoEditor({
  value,
  language,
  onChange,
  readOnly = false,
  height = "100%",
}: MonacoEditorProps) {
  const containerRef = useRef<HTMLDivElement>(null);
  const editorRef = useRef<monacoEditor.IStandaloneCodeEditor | null>(null);

  useEffect(() => {
    if (!containerRef.current) { return; }

    let disposed = false;
    let editor: monacoEditor.IStandaloneCodeEditor | null = null;

    import("monaco-editor").then((monaco) => {
      if (disposed || !containerRef.current) { return; }
      editor = monaco.editor.create(containerRef.current, {
        value,
        language: LANGUAGE_MAP[language] || "plaintext",
        readOnly,
        theme: "vs-dark",
        minimap: { enabled: false },
        fontSize: 13,
        lineNumbers: "on",
        scrollBeyondLastLine: false,
        automaticLayout: true,
        wordWrap: "on",
        padding: { top: 8 },
      });
      editorRef.current = editor;

      if (onChange) {
        editor.onDidChangeModelContent(() => {
          onChange(editor!.getValue());
        });
      }
    });

    return () => {
      disposed = true;
      editor?.dispose();
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  useEffect(() => {
    const editor = editorRef.current;
    if (editor) {
      const model = editor.getModel();
      if (model && model.getValue() !== value) {
        editor.setValue(value);
      }
    }
  }, [value]);

  useEffect(() => {
    import("monaco-editor").then((monaco) => {
      const editor = editorRef.current;
      if (editor) {
        const model = editor.getModel();
        if (model) {
          monaco.editor.setModelLanguage(model, LANGUAGE_MAP[language] || "plaintext");
        }
      }
    });
  }, [language]);

  return <div ref={containerRef} style={{ height, width: "100%" }} />;
}
