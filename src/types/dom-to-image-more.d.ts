// SPDX-License-Identifier: AGPL-3.0-only

declare module "dom-to-image-more" {
  /**
   * Minimal ambient typings for `dom-to-image-more` (no first-party types shipped).
   * Only the surface we actually use is declared.
   */
  export interface DomToImageOptions {
    /** 背景色，缺省不指定 */
    bgColor?: string;
    /** 渲染时使用的 device pixel ratio */
    scale?: number;
    /** 节点相对/绝对宽高（可选） */
    width?: number;
    height?: number;
    /** 注入的样式表，可选 */
    style?: Record<string, string>;
    /** 节点过滤器：返回 false 可剔除该节点 */
    filter?: (node: Node) => boolean;
  }

  function _toBlob(node: HTMLElement, options?: DomToImageOptions): Promise<Blob | null>;
  function _toPng(node: HTMLElement, options?: DomToImageOptions): Promise<string>;
  function _toJpeg(node: HTMLElement, options?: DomToImageOptions): Promise<string>;
  function _toSvg(node: HTMLElement, options?: DomToImageOptions): Promise<string>;
  function _toPixelData(node: HTMLElement, options?: DomToImageOptions): Promise<Uint8ClampedArray>;

  const _default: {
    toBlob: typeof _toBlob;
    toPng: typeof _toPng;
    toJpeg: typeof _toJpeg;
    toSvg: typeof _toSvg;
    toPixelData: typeof _toPixelData;
  };
  export default _default;
}
