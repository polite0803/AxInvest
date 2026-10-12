// SPDX-License-Identifier: AGPL-3.0-only

import { Button, InputNumber, Radio, Typography } from "antd";
import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { useTranslation } from "react-i18next";

/**
 * 分段式可视化 Cron 构建器（受控，value/onChange 暴露 cron 字符串）。
 *
 * 按 cron 的五个字段逐段编辑，任意标准 5 段表达式都能纯鼠标配出：
 *   分钟 / 小时 / 日期 / 月份 / 星期
 * 每段三种取法：
 *   - any    每个（*）
 *   - step   周期（"*" 与 "/" 连用表示步进、S-MAX/N 为带起始的步进）
 *   - list   指定（网格多选：1,15,30 / 1-5 …）
 * 底部只读预览：最终表达式 + 未来 3 次执行时间。
 * 遇到含 L/W/#/? 等扩展词法、无法可视化反解析的表达式，自动降级为原始文本编辑，绝不丢数据。
 */

// ── 数据模型 ────────────────────────────────────────────────────────────────

type FieldMode = "any" | "step" | "list";

export interface FieldState {
  mode: FieldMode;
  /** step：步长 */
  interval: number;
  /** step：起始值 */
  start: number;
  /** list：选中的值（内部无序，序列化时排序） */
  selected: number[];
}

export interface CronFields {
  minute: FieldState;
  hour: FieldState;
  dom: FieldState;
  month: FieldState;
  dow: FieldState;
}

const makeField = (mode: FieldMode = "any", min: number): FieldState => ({
  mode,
  interval: 1,
  start: min,
  selected: [min],
});

const DEFAULT_FIELDS: CronFields = {
  // 默认每天 09:00（后端禁止每分钟执行，不能给 minute=any 的默认值）
  minute: { mode: "list", interval: 1, start: 0, selected: [0] },
  hour: { mode: "list", interval: 1, start: 9, selected: [9] },
  dom: makeField("any", 1),
  month: makeField("any", 1),
  dow: makeField("any", 0),
};

// ── 词法展开（反解析与未来时间计算共用） ───────────────────────────────────

/** 展开单个 cron 字段为允许值集合；非法语法返回 null */
function expandField(raw: string, min: number, max: number): Set<number> | null {
  const result = new Set<number>();
  for (const part of raw.split(",")) {
    let step = 1;
    let body = part;
    const slash = part.split("/");
    if (slash.length > 2) {
      return null;
    }
    if (slash.length === 2) {
      if (!/^\d+$/.test(slash[1])) {
        return null;
      }
      step = Number(slash[1]);
      if (step < 1) {
        return null;
      }
      body = slash[0];
    }
    let lo = min;
    let hi = max;
    if (body !== "*") {
      const range = body.split("-");
      if (range.length === 1) {
        if (!/^\d+$/.test(range[0])) {
          return null;
        }
        lo = Number(range[0]);
        // a/n 语法：从 a 到段末按步进
        hi = slash.length === 2 ? max : lo;
      } else if (range.length === 2) {
        if (!/^\d+$/.test(range[0]) || !/^\d+$/.test(range[1])) {
          return null;
        }
        lo = Number(range[0]);
        hi = Number(range[1]);
      } else {
        return null;
      }
    }
    if (lo < min || hi > max || lo > hi) {
      return null;
    }
    for (let v = lo; v <= hi; v += step) {
      result.add(v);
    }
  }
  return result.size > 0 ? result : null;
}

function parseField(raw: string, min: number, max: number): FieldState | null {
  const value = raw.trim();
  if (value === "*") {
    return makeField("any", min);
  }
  // */N：从段首开始的步进
  let m = /^\*\/(\d+)$/.exec(value);
  if (m) {
    return { mode: "step", interval: Number(m[1]), start: min, selected: [min] };
  }
  // S/N 或 S-END/N：带起始值的步进（END 不保留，步进口径统一到段末，序列化时重写）
  m = /^(\d+)(?:-\d+)?\/(\d+)$/.exec(value);
  if (m) {
    const start = Number(m[1]);
    if (start < min || start > max) {
      return null;
    }
    return { mode: "step", interval: Number(m[2]), start, selected: [start] };
  }
  // 其余形态（纯数字、逗号列表、显式范围 1-5、混合）：忠实展开为网格点选
  const values = expandField(value, min, max);
  if (!values) {
    return null;
  }
  return {
    mode: "list",
    interval: 1,
    start: min,
    selected: Array.from(values).sort((a, b) => a - b),
  };
}

/** 解析完整 5 段表达式；任一字段含不可识别词法（L/W/# 等）返回 null */
export function parseCronFields(cron: string | null | undefined): CronFields | null {
  if (!cron) {
    return null;
  }
  const parts = cron.trim().split(/\s+/);
  if (parts.length !== 5) {
    return null;
  }
  const minute = parseField(parts[0], 0, 59);
  const hour = parseField(parts[1], 0, 23);
  const dom = parseField(parts[2], 1, 31);
  const month = parseField(parts[3], 1, 12);
  const dow = parseField(normalizeDow(parts[4]), 0, 6);
  if (!minute || !hour || !dom || !month || !dow) {
    return null;
  }
  return { minute, hour, dom, month, dow };
}

/**
 * 星期字段归一：Vixie cron 中周日可写作 0 或 7，把独立的 7 归一为 0
 * （裸 7 与列表如 1,7 均可；跨周范围 5-7 不支持，仍会降级 raw）。
 */
function normalizeDow(raw: string): string {
  return raw.replace(/(^|[^0-9])7(?![0-9])/g, (_m, prefix: string) => `${prefix}0`);
}

function serializeField(f: FieldState, min: number, max: number): string {
  if (f.mode === "any") {
    return "*";
  }
  if (f.mode === "step") {
    const interval = Math.min(Math.max(1, Math.trunc(f.interval)), max - min + 1);
    const start = Math.min(Math.max(min, Math.trunc(f.start)), max);
    return start === min ? `*/${interval}` : `${start}-${max}/${interval}`;
  }
  const selected = Array.from(new Set(f.selected))
    .filter((v) => v >= min && v <= max)
    .sort((a, b) => a - b);
  if (selected.length === 0) {
    return "*";
  }
  // 连续段压缩为 a-b（点选 9..18 与手写 9-18 序列化结果一致，表达式更短）
  const tokens: string[] = [];
  let rangeStart = selected[0];
  let prev = selected[0];
  const flush = (end: number) => {
    tokens.push(end === rangeStart ? String(end) : `${rangeStart}-${end}`);
  };
  for (let i = 1; i < selected.length; i++) {
    const v = selected[i];
    if (v === prev + 1) {
      prev = v;
      continue;
    }
    flush(prev);
    rangeStart = v;
    prev = v;
  }
  flush(prev);
  return tokens.join(",");
}

export function serializeFields(fields: CronFields): string {
  return [
    serializeField(fields.minute, 0, 59),
    serializeField(fields.hour, 0, 23),
    serializeField(fields.dom, 1, 31),
    serializeField(fields.month, 1, 12),
    serializeField(fields.dow, 0, 6),
  ].join(" ");
}

/**
 * 前端 cron 校验，规则与后端 validate_cron_expression 对齐：
 * 必须为 5 段；不允许每分钟执行（最小间隔 2 分钟）。
 * @returns 错误文案 i18n key；合法返回 null
 */
export function validateCronSyntax(cron: string): string | null {
  const value = cron.trim();
  if (!value) {
    return "settings.scheduler.cronRequired";
  }
  const parts = value.split(/\s+/);
  if (parts.length !== 5) {
    return "settings.scheduler.cronInvalid";
  }
  // 词法必须全部可反解析（拦截 L/W/#/? 等扩展及乱填）
  if (!parseCronFields(value)) {
    return "settings.scheduler.cronInvalid";
  }
  // 语义判断「每分钟执行」：分钟字段覆盖 0-59 全集
  // （字面 *、*/1、0-59/1、网格全选 60 格均在此列，与后端最小 2 分钟口径对齐）
  const minuteValues = expandField(parts[0], 0, 59);
  if (minuteValues && minuteValues.size === 60) {
    return "settings.scheduler.cronTooFrequent";
  }
  return null;
}

// ── 未来执行时间计算 ────────────────────────────────────────────────────────

/**
 * 计算从现在起未来 count 次触发时间。逐分钟扫描（上限 366 天）；
 * 表达式非法或一年内无触发返回空数组。
 * day-of-month 与 day-of-week 同时受限时按标准 cron 的 OR 语义处理。
 */
export function nextRuns(cron: string, count = 3): Date[] {
  const parts = cron.trim().split(/\s+/);
  if (parts.length !== 5) {
    return [];
  }
  const minutes = expandField(parts[0], 0, 59);
  const hours = expandField(parts[1], 0, 23);
  const doms = expandField(parts[2], 1, 31);
  const months = expandField(parts[3], 1, 12);
  // 星期字段：0 与 7 均为周日（7 先归一为 0），内部统一成 JS getDay()（0=周日）
  const dowRaw = normalizeDow(parts[4]);
  let dows: Set<number> | null;
  if (dowRaw === "*") {
    dows = null;
  } else {
    dows = new Set();
    for (const token of dowRaw.split(",")) {
      let step = 1;
      let body = token;
      const slash = token.split("/");
      if (slash.length === 2) {
        if (!/^\d+$/.test(slash[1])) {
          return [];
        }
        step = Number(slash[1]);
        body = slash[0];
      }
      let lo = 0;
      let hi = 6;
      if (body !== "*") {
        const range = body.split("-");
        if (range.length === 1) {
          if (!/^\d+$/.test(range[0])) {
            return [];
          }
          lo = Number(range[0]) % 7;
          hi = slash.length === 2 ? 6 : lo;
        } else if (range.length === 2) {
          if (!/^\d+$/.test(range[0]) || !/^\d+$/.test(range[1])) {
            return [];
          }
          lo = Number(range[0]) % 7;
          hi = Number(range[1]) % 7;
        } else {
          return [];
        }
      }
      if (lo > hi) {
        return [];
      }
      for (let v = lo; v <= hi; v += step) {
        dows.add(v % 7);
      }
    }
  }
  if (!minutes || !hours || !doms || !months) {
    return [];
  }

  const domRestricted = parts[2] !== "*";
  const dowRestricted = dows !== null;

  const cursor = new Date();
  cursor.setSeconds(0, 0);
  cursor.setMinutes(cursor.getMinutes() + 1);

  const hits: Date[] = [];
  const maxIterations = 366 * 24 * 60;
  for (let i = 0; i < maxIterations && hits.length < count; i++) {
    const month = cursor.getMonth() + 1;
    if (months.has(month)) {
      const domOk = doms.has(cursor.getDate());
      const dowOk = !dowRestricted || dows!.has(cursor.getDay());
      // 两者同时受限取 OR（Vixie cron 语义），否则取 AND
      const dayMatch = domRestricted && dowRestricted ? domOk || dowOk : domOk && dowOk;
      if (dayMatch && hours.has(cursor.getHours()) && minutes.has(cursor.getMinutes())) {
        hits.push(new Date(cursor));
      }
    }
    cursor.setMinutes(cursor.getMinutes() + 1);
  }
  return hits;
}

// ── 单段编辑器 ──────────────────────────────────────────────────────────────

interface FieldEditorProps {
  label: string;
  field: FieldState;
  min: number;
  max: number;
  /** 网格展示值（默认 min..max）；星期字段用它做周一开始的排序与本地化标签 */
  values?: number[];
  valueLabel?: (v: number) => string;
  /** 网格最大高度（分钟 60 格需要滚动） */
  gridMaxHeight?: number;
  /** 数字是否补零两位（默认 max≥10 时补零；月份网格显式关闭） */
  pad?: boolean;
  onChange: (next: FieldState) => void;
}

function FieldEditor({
  label,
  field,
  min,
  max,
  values,
  valueLabel,
  gridMaxHeight,
  pad: padProp,
  onChange,
}: FieldEditorProps) {
  const { t } = useTranslation();
  const options = useMemo(() => values ?? Array.from({ length: max - min + 1 }, (_, i) => min + i), [values, min, max]);
  const usePad = padProp ?? max >= 10;
  const labelOf = (v: number) => valueLabel ? valueLabel(v) : usePad ? String(v).padStart(2, "0") : String(v);

  const selectedSet = useMemo(() => new Set(field.selected), [field.selected]);

  const toggle = (v: number) => {
    const next = new Set(selectedSet);
    if (next.has(v)) {
      next.delete(v);
    } else {
      next.add(v);
    }
    onChange({ ...field, selected: Array.from(next).sort((a, b) => a - b) });
  };

  const setAll = () => onChange({ ...field, selected: options.slice().sort((a, b) => a - b) });
  const clear = () => onChange({ ...field, selected: [] });

  return (
    <div
      style={{
        display: "flex",
        gap: 12,
        alignItems: "flex-start",
        padding: "6px 0",
        borderBottom: "1px solid var(--color-border-secondary, rgba(255,255,255,0.06))",
      }}
    >
      <div
        style={{
          width: 44,
          flex: "0 0 44px",
          paddingTop: 5,
          color: "var(--color-text-secondary)",
          fontSize: 13,
          textAlign: "right",
        }}
      >
        {label}
      </div>

      <div style={{ flex: 1, minWidth: 0 }}>
        <Radio.Group
          value={field.mode}
          onChange={(e) => {
            const mode = e.target.value as FieldMode;
            onChange({
              ...field,
              mode,
              // 切到步进/指定时给一个合法起始选择，避免空态困惑
              selected: field.selected.length ? field.selected : [field.start >= min ? field.start : min],
            });
          }}
          optionType="button"
          buttonStyle="solid"
          size="small"
        >
          <Radio.Button value="any">{t("settings.scheduler.cronOptAny")}</Radio.Button>
          <Radio.Button value="step">{t("settings.scheduler.cronOptStep")}</Radio.Button>
          <Radio.Button value="list">{t("settings.scheduler.cronOptSpecify")}</Radio.Button>
        </Radio.Group>

        {field.mode === "step" && (
          <div style={{ display: "flex", gap: 8, alignItems: "center", marginTop: 8, flexWrap: "wrap" }}>
            <span style={{ color: "var(--color-text-secondary)", fontSize: 13 }}>
              {t("settings.scheduler.cronStepInterval")}
            </span>
            <InputNumber
              size="small"
              min={1}
              max={max - min + 1}
              value={field.interval}
              onChange={(v) =>
                onChange({ ...field, interval: v ?? 1 })}
              style={{ width: 72 }}
            />
            <span style={{ color: "var(--color-text-secondary)", fontSize: 13 }}>
              {t("settings.scheduler.cronStepStart")}
            </span>
            <InputNumber
              size="small"
              min={min}
              max={max}
              value={field.start}
              onChange={(v) => onChange({ ...field, start: v ?? min })}
              style={{ width: 72 }}
            />
          </div>
        )}

        {field.mode === "list" && (
          <div style={{ marginTop: 8 }}>
            <div
              style={{
                display: "grid",
                gridTemplateColumns: `repeat(auto-fill, minmax(34px, 1fr))`,
                gap: 4,
                maxHeight: gridMaxHeight,
                overflowY: gridMaxHeight ? "auto" : undefined,
                paddingRight: 4,
              }}
            >
              {options.map((v) => {
                const active = selectedSet.has(v);
                return (
                  <button
                    key={v}
                    type="button"
                    onClick={() => toggle(v)}
                    style={{
                      height: 24,
                      padding: 0,
                      fontSize: 11,
                      lineHeight: "22px",
                      borderRadius: 4,
                      cursor: "pointer",
                      border: active
                        ? "1px solid var(--ant-color-primary, #1677ff)"
                        : "1px solid var(--color-border, #434343)",
                      background: active
                        ? "var(--ant-color-primary, #1677ff)"
                        : "transparent",
                      color: active ? "#fff" : "var(--color-text, rgba(255,255,255,0.85))",
                    }}
                  >
                    {labelOf(v)}
                  </button>
                );
              })}
            </div>
            <div style={{ display: "flex", gap: 12, marginTop: 6 }}>
              <Button size="small" type="link" style={{ padding: 0 }} onClick={setAll}>
                {t("settings.scheduler.cronSelectAll")}
              </Button>
              <Button size="small" type="link" style={{ padding: 0 }} onClick={clear}>
                {t("settings.scheduler.cronClear")}
              </Button>
              {field.selected.length === 0 && (
                <span style={{ fontSize: 12, color: "var(--color-error, #ff4d4f)" }}>
                  {t("settings.scheduler.cronSelectRequired")}
                </span>
              )}
            </div>
          </div>
        )}
      </div>
    </div>
  );
}

// ── 主组件 ──────────────────────────────────────────────────────────────────

export interface CronScheduleBuilderProps {
  value?: string;
  onChange?: (cron: string) => void;
}

/** 星期网格顺序：周一开始，周日收尾（值仍为 cron 数字 1-6,0） */
const WEEKDAY_ORDER = [1, 2, 3, 4, 5, 6, 0];

/** cron 星期数字（0=周日）→ i18n 键后缀 */
const DOW_I18N_KEYS = [
  "sunday",
  "monday",
  "tuesday",
  "wednesday",
  "thursday",
  "friday",
  "saturday",
] as const;

export function CronScheduleBuilder({ value = "", onChange }: CronScheduleBuilderProps) {
  const { t } = useTranslation();
  const [fields, setFields] = useState<CronFields>(DEFAULT_FIELDS);
  const [rawMode, setRawMode] = useState(false);
  const [raw, setRaw] = useState(value || "");
  /** 最近一次由组件内部产出的 cron；避免反解析外部值时与自身回写打架 */
  const lastEmittedRef = useRef<string>("");

  useEffect(() => {
    if (value === lastEmittedRef.current) {
      return;
    }
    if (!value) {
      // 空值（新建瞬间）：给可视化默认形态，不要误降级为 raw 文本框
      setFields(DEFAULT_FIELDS);
      setRawMode(false);
      return;
    }
    const parsed = parseCronFields(value);
    if (parsed) {
      setFields(parsed);
      setRawMode(false);
    } else {
      // 含扩展词法的表达式无法可视化：降级为原始文本编辑，原样保留
      setRawMode(true);
      setRaw(value);
    }
  }, [value]);

  const patchField = useCallback(
    (key: keyof CronFields, next: FieldState) => {
      setFields((prev) => {
        const updated = { ...prev, [key]: next };
        const cron = serializeFields(updated);
        lastEmittedRef.current = cron;
        onChange?.(cron);
        return updated;
      });
    },
    [onChange],
  );

  const switchFromRaw = () => {
    const parsed = parseCronFields(raw);
    if (parsed) {
      setFields(parsed);
      setRawMode(false);
      const cron = serializeFields(parsed);
      lastEmittedRef.current = cron;
      onChange?.(cron);
    }
  };

  const cron = rawMode ? raw.trim() : serializeFields(fields);
  const syntaxError = validateCronSyntax(cron);
  const previewRuns = useMemo(() => (syntaxError ? [] : nextRuns(cron, 3)), [cron, syntaxError]);

  return (
    <div style={{ display: "flex", flexDirection: "column", gap: 8 }}>
      {rawMode
        ? (
          <div>
            <div style={{ fontSize: 12, color: "var(--color-warning, #faad14)", marginBottom: 6 }}>
              {t("settings.scheduler.cronRawUnsupported")}
            </div>
            <input
              value={raw}
              onChange={(e) => {
                const next = e.target.value;
                setRaw(next);
                lastEmittedRef.current = next.trim();
                onChange?.(next.trim());
              }}
              placeholder="*/15 9-18 * * 1-5"
              autoComplete="off"
              spellCheck={false}
              style={{
                width: "100%",
                padding: "4px 11px",
                fontFamily: "var(--font-mono, monospace)",
                background: "var(--color-bg-container, #1f1f1f)",
                color: "var(--color-text, rgba(255,255,255,0.88))",
                border: `1px solid ${
                  syntaxError
                    ? "var(--color-error, #ff4d4f)"
                    : "var(--color-border, #434343)"
                }`,
                borderRadius: 6,
                outline: "none",
              }}
            />
            <div style={{ marginTop: 6 }}>
              <Button size="small" onClick={switchFromRaw} disabled={!parseCronFields(raw)}>
                {t("settings.scheduler.cronBackToVisual")}
              </Button>
            </div>
          </div>
        )
        : (
          <div>
            <FieldEditor
              label={t("settings.scheduler.cronFMinute")}
              field={fields.minute}
              min={0}
              max={59}
              gridMaxHeight={112}
              onChange={(f) => patchField("minute", f)}
            />
            <FieldEditor
              label={t("settings.scheduler.cronFHour")}
              field={fields.hour}
              min={0}
              max={23}
              onChange={(f) => patchField("hour", f)}
            />
            <FieldEditor
              label={t("settings.scheduler.cronFDay")}
              field={fields.dom}
              min={1}
              max={31}
              onChange={(f) => patchField("dom", f)}
            />
            <FieldEditor
              label={t("settings.scheduler.cronFMonth")}
              field={fields.month}
              min={1}
              max={12}
              pad={false}
              onChange={(f) => patchField("month", f)}
            />
            <FieldEditor
              label={t("settings.scheduler.cronFWeekday")}
              field={fields.dow}
              min={0}
              max={6}
              values={WEEKDAY_ORDER}
              valueLabel={(v) => {
                // 中文「周一」在网格按钮里收窄为「一」；其余语言本身已是短形（Mon/Mo/月…）
                const full = t(`settings.scheduler.${DOW_I18N_KEYS[v]}`);
                // 首字判据用码点写（周 U+5468 / 日文週 U+9031），避免 CJK 字面量
                return /^[\u5468\u9031]/.test(full) ? full.slice(1) : full;
              }}
              onChange={(f) => patchField("dow", f)}
            />
          </div>
        )}

      {/* 只读预览：最终表达式 + 未来执行时间 */}
      <div
        style={{
          padding: "8px 12px",
          borderRadius: 6,
          background: "var(--color-fill-quaternary, rgba(255,255,255,0.04))",
          fontSize: 12,
        }}
      >
        <div style={{ display: "flex", gap: 8, alignItems: "center", flexWrap: "wrap" }}>
          <Typography.Text type="secondary" style={{ fontSize: 12 }}>
            {t("settings.scheduler.cronExpression")}:
          </Typography.Text>
          <Typography.Text
            code
            copyable={{ text: cron }}
            style={{ color: syntaxError ? "var(--color-error, #ff4d4f)" : undefined }}
          >
            {cron || "-"}
          </Typography.Text>
          {syntaxError && (
            <Typography.Text style={{ fontSize: 12, color: "var(--color-error, #ff4d4f)" }}>
              {t(syntaxError)}
            </Typography.Text>
          )}
        </div>
        {!syntaxError
          && (previewRuns.length > 0
            ? (
              <div style={{ marginTop: 4 }}>
                <Typography.Text type="secondary" style={{ fontSize: 12 }}>
                  {t("settings.scheduler.cronNextRuns")}:
                </Typography.Text>
                <ul style={{ margin: "2px 0 0 18px", padding: 0 }}>
                  {previewRuns.map((d, i) => <li key={i}>{d.toLocaleString()}</li>)}
                </ul>
              </div>
            )
            : (
              <Typography.Text type="secondary" style={{ fontSize: 12 }}>
                {t("settings.scheduler.cronNoNextRun")}
              </Typography.Text>
            ))}
      </div>
    </div>
  );
}
