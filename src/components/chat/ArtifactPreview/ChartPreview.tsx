// SPDX-License-Identifier: AGPL-3.0-only

import { useIsDarkTheme } from "@/hooks/useIsDarkTheme";
import { cssVar } from "@/lib/cssVar";
import { hueColor, SERIES_HUES } from "@/theme/categoricalColors";
import { memo, useEffect, useMemo, useRef } from "react";
import { useTranslation } from "react-i18next";

interface ChartPreviewProps {
  option: Record<string, unknown>;
  width?: number | string;
  height?: number | string;
}

export const ChartPreview = memo(function ChartPreview({
  option,
  width,
  height,
}: ChartPreviewProps) {
  const { t } = useTranslation();
  const iframeRef = useRef<HTMLIFrameElement>(null);
  const isDark = useIsDarkTheme();
  // 系列色取自类目色板（宿主侧按明暗档解析成具体值后注入 iframe）
  const palette = useMemo(() => SERIES_HUES.map((hue) => hueColor(hue, isDark)), [isDark]);

  useEffect(() => {
    if (iframeRef.current) {
      // iframe 是独立文档，看不到宿主的 CSS 变量 ⇒ 在宿主侧把 token 解析成具体色值再注入。
      // 兜底不再是「另一套写死的深色」，而是宿主主题的真实底色/前景色。
      const bgColor = cssVar("--bg");
      const textColor = cssVar("--fg");

      iframeRef.current.srcdoc = `<!DOCTYPE html>
<html>
<head>
<meta charset="utf-8">
<script src="https://cdn.jsdelivr.net/npm/echarts@5.5.0/dist/echarts.min.js" integrity="sha384-o5uz97et3bErHvpKfD4Jz4n0JfhJDWABFuF4NP+iEEDxE1VwMWJ19QGR0lqFZnr6" crossorigin="anonymous"></script>
<style>
  body { margin: 0; background: ${bgColor}; }
  #chart { width: 100%; height: 100%; }
</style>
</head>
<body>
<div id="chart"></div>
<script>
var chart = echarts.init(document.getElementById('chart'), null, { renderer: 'canvas' });
var option = ${JSON.stringify(option)};
option.color = option.color || ${JSON.stringify(palette)};
if (!option.textStyle) option.textStyle = { color: '${textColor}' };
chart.setOption(option);
window.addEventListener('resize', function() { chart.resize(); });
</script>
</body>
</html>`;
    }
  }, [option, palette]);

  return (
    <iframe
      ref={iframeRef}
      sandbox="allow-scripts"
      title={t("chartPreview.title")}
      style={{
        width: width || "100%",
        height: height || 400,
        border: "none",
        borderRadius: 8,
      }}
    />
  );
});
