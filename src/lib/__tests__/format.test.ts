import { describe, expect, it } from "vitest";

import i18n from "@/i18n";
import { formatYi } from "@/lib/format";

/**
 * 语言包按需加载（`src/i18n/index.ts` 的 `loadLocale`）—— 测试环境里只有缺省语言是就绪的，
 * 其余语言 `t()` 会**原样返回键名**，那样上面的断言全都只是字符串比对、测不到数制。
 * 所以这里按生产同一条路径把资源挂上去（`addResourceBundle`），不许用 mock 替身代替真资源。
 */
async function ready(lng: string) {
  if (i18n.hasResourceBundle(lng, "translation")) { return; }
  const mod = await import(`../../i18n/locales/${lng}.json`);
  i18n.addResourceBundle(lng, "translation", mod.default, true, true);
}

/**
 * #47 的锁：数制按语言分族，且**中文两支读数逐字不变**。
 *
 * 为什么值得单独锁：旧实现只有 `亿/万` 一档口径，改成多族时最容易犯两种错——
 * ① 只换单位词不换因子（把 `亿`=1e8 直译成 en 的 "B"=1e9 ⇒ 错一个数量级，
 *    数字看着仍像话，肉眼与快照都难发现）；
 * ② 把日/韩当成「非中文 ⇒ 改短 scale」（它们本就同用 1e8/1e4 万字族 ⇒ 现网读数被改）。
 * 所以下面既锁中文快照，也锁「非中文 locale 不得出现 `亿|万`」。
 */
describe("formatYi 按语言数制", () => {
  it("zh-CN / zh-TW 与旧实现逐字相同（现网行情读数不变）", async () => {
    await ready("zh-CN");
    await ready("zh-TW");

    expect(formatYi(123456789, "zh-CN")).toBe("1.23亿");
    expect(formatYi(12345678, "zh-CN")).toBe("1235万");
    expect(formatYi(890, "zh-CN")).toBe("890");
    expect(formatYi(-123456789, "zh-CN")).toBe("-1.23亿");
    expect(formatYi(123456789, "zh-TW")).toBe("1.23億");
    expect(formatYi(12345678, "zh-TW")).toBe("1235萬");
  });

  it("日/韩同属万字族（不是「非中文就改短 scale」）", async () => {
    await ready("ja");
    await ready("ko");

    expect(formatYi(123456789, "ja")).toContain("億");
    expect(formatYi(12345678, "ja")).toContain("万");
    expect(formatYi(123456789, "ko")).toContain("억");
  });

  it("短 scale 族按 1e9/1e6 换算，且**不得**出现中文单位", async () => {
    for (const l of ["en-US", "de", "es", "fr", "ru", "ar"]) { await ready(l); }

    expect(formatYi(1_234_567_890, "en-US")).toBe("1.23B");
    expect(formatYi(5_600_000, "en-US")).toBe("5.60M");
    for (const loc of ["en-US", "de", "es", "fr", "ru", "ar"]) {
      const out = formatYi(1_234_567_890, loc) + formatYi(12_345_678, loc);
      expect(out, `${loc} 单位键没解析出资源（拿到键名就是在测 mock）`).not.toContain("stockAnalysis");
      expect(out, `${loc} 串进了中文单位`).not.toMatch(/[亿万]/);
    }
  });

  it("印地语走 1e7/1e5 族（把 करोड़ 当 1e6 是数制错误）", async () => {
    await ready("hi");

    expect(formatYi(23_400_000, "hi")).toContain("करोड़");
    expect(formatYi(350_000, "hi")).toContain("लाख");
    // 同一数额在短 scale 族是 23.40 M、在印度族是 2.34 करोड़ ⇒ 因子真的换了，不是只换词
    expect(formatYi(23_400_000, "hi").replace(/[^\d.]/g, "")).toBe("2.34");
    expect(formatYi(23_400_000, "en-US").replace(/[^\d.]/g, "")).toBe("23.40");
  });

  it("未知 locale 回落万字族：换语言不许改数字，只可能改单位词", async () => {
    await ready("zh-CN");

    expect(formatYi(123456789, "zz-ZZ")).toBe("1.23亿");
  });
});
