// SPDX-License-Identifier: AGPL-3.0-only

import type { AttachmentInput } from "@/types";
import { File, FileText, Film, Image as ImageIcon, Music } from "lucide-react";

export type FileTypeCategory = "image" | "video" | "audio" | "document" | "other";

export async function fileToAttachmentInput(file: File): Promise<AttachmentInput> {
  return new Promise((resolve, reject) => {
    const reader = new FileReader();
    reader.onload = () => {
      const base64 = (reader.result as string).split(",")[1] || "";
      resolve({
        fileName: file.name,
        fileType: file.type || "application/octet-stream",
        fileSize: file.size,
        data: base64,
      });
    };
    reader.onerror = () => {
      reject(new Error(`Failed to read file: ${file.name}`));
    };
    reader.readAsDataURL(file);
  });
}

export function getFileTypeCategory(mimeType: string): FileTypeCategory {
  if (mimeType.startsWith("image/")) {
    return "image";
  }
  if (mimeType.startsWith("video/")) {
    return "video";
  }
  if (mimeType.startsWith("audio/")) {
    return "audio";
  }
  if (
    mimeType.startsWith("text/")
    || mimeType === "application/pdf"
    || mimeType.includes("document")
    || mimeType.includes("spreadsheet")
    || mimeType.includes("presentation")
    || mimeType.includes("word")
  ) {
    return "document";
  }
  return "other";
}

// ── formatFileSize 已收敛到 @/lib/format（唯一实现，禁止在本文件重复定义）──
// 口径统一：一律保留 1 位小数（`1.0 KB`），单位表到 PB，且 null/undefined 返回 "—"。
// 原实现用 `parseFloat(toFixed(1))` 去尾零并截断在 GB，属可见口径分歧，2026-09-13 已统一。

export function getFileIcon(category: FileTypeCategory) {
  switch (category) {
    case "image":
      return <ImageIcon size={16} />;
    case "video":
      return <Film size={16} />;
    case "audio":
      return <Music size={16} />;
    case "document":
      return <FileText size={16} />;
    default:
      return <File size={16} />;
  }
}

/** 缩写工作目录路径：保留盘符 + 最后 3 段，超长路径省略中间部分 */
export function abbreviatePath(path: string): string {
  const normalized = path.replace(/\\/g, "/");
  const segments = normalized.split("/").filter(Boolean);
  if (segments.length <= 3 || normalized.length <= 45) {
    return path;
  }
  // 保留盘符（如 D:）+ 最后 3 段
  const drive = segments[0].endsWith(":") ? segments[0] : null;
  const tail = segments.slice(-3);
  const abbreviated = drive
    ? [drive, "…", ...tail].join("/")
    : "…/" + tail.join("/");
  return abbreviated;
}
