import { chromium } from "playwright";

let browser = null;
let page = null;

async function init() {
  browser = await chromium.launch({ headless: true });
  const context = await browser.newContext({
    viewport: { width: 1280, height: 720 },
    locale: "zh-CN",
  });
  page = await context.newPage();
}

process.stdin.on("data", async (data) => {
  const msg = JSON.parse(data.toString().trim());
  let result;

  try {
    switch (msg.method) {
      case "navigate": {
        await page.goto(msg.params.url, { waitUntil: "domcontentloaded", timeout: 30000 });
        result = { url: page.url(), title: await page.title() };
        break;
      }
      case "screenshot": {
        const buffer = await page.screenshot({ type: "png", fullPage: msg.params.fullPage || false });
        result = { image_base64: buffer.toString("base64") };
        break;
      }
      case "click": {
        await page.click(msg.params.selector, { timeout: 10000 });
        result = { success: true };
        break;
      }
      case "fill": {
        await page.fill(msg.params.selector, msg.params.value);
        result = { success: true };
        break;
      }
      case "type": {
        await page.locator(msg.params.selector).pressSequentially(msg.params.text, { delay: 50 });
        result = { success: true };
        break;
      }
      case "select": {
        await page.selectOption(msg.params.selector, msg.params.value);
        result = { success: true };
        break;
      }
      case "extract_text": {
        const text = await page.locator(msg.params.selector).textContent();
        result = { text };
        break;
      }
      case "extract_all": {
        const elements = await page.$$eval(msg.params.selector, (els) =>
          els.map((el) => ({
            tag: el.tagName.toLowerCase(),
            text: el.textContent?.trim().slice(0, 200),
            href: el.getAttribute("href"),
            type: el.getAttribute("type"),
            placeholder: el.getAttribute("placeholder"),
          })));
        result = { elements, count: elements.length };
        break;
      }
      case "wait_for": {
        await page.waitForSelector(msg.params.selector, { timeout: msg.params.timeout || 10000 });
        result = { success: true };
        break;
      }
      case "get_content": {
        const html = await page.content();
        result = { html: html.slice(0, 100000) };
        break;
      }
      case "http_get": {
        // 通过页面导航发送 GET 请求绕过 CORS 和 TLS 指纹限制
        // 返回 body + 诊断信息（实际 URL、title、body 前 300 字符）
        const prevUrl = page.url();
        let navigatedUrl = "";
        let pageTitle = "";
        let body = "";
        let contentType = "";
        try {
          const resp = await page.goto(msg.params.url, {
            waitUntil: "domcontentloaded",
            timeout: 20000,
          });
          navigatedUrl = page.url();
          pageTitle = await page.title();
          if (resp) {
            contentType = resp.headers()["content-type"] || "";
          }
          body = await page.evaluate(() => document.body.innerText);
          // 截取前 3000 字符防止回传过大
          if (body.length > 3000) {
            body = body.slice(0, 3000);
          }
        } catch (navErr) {
          navigatedUrl = page.url();
          body = `PAGE_GOTO_ERROR: ${navErr.message}`;
        }
        // 回退到原页面
        if (prevUrl && prevUrl !== "about:blank") {
          page.goBack().catch(() => {});
        }
        result = { body, navigatedUrl, pageTitle, contentType };
        break;
      }
      case "evaluate": {
        // 在浏览器上下文中执行任意 JS 代码，返回序列化结果
        result = await page.evaluate(msg.params.code);
        break;
      }
      case "close": {
        await browser.close();
        result = { success: true };
        break;
      }
      default:
        throw new Error(`Unknown method: ${msg.method}`);
    }

    process.stdout.write(JSON.stringify({ id: msg.id, result }) + "\n");
  } catch (error) {
    process.stdout.write(JSON.stringify({ id: msg.id, error: error.message }) + "\n");
  }
});

init().then(() => {
  process.stdout.write(JSON.stringify({ ready: true }) + "\n");
});
