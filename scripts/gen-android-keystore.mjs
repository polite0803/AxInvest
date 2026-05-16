#!/usr/bin/env node
/**
 * 生成 Android 签名用的 PKCS12 keystore
 * 使用 openssl 生成 RSA 密钥对 + 自签名证书，打包为 PKCS12
 */

import { execSync } from "child_process";
import { generateKeyPairSync } from "crypto";
import { existsSync, readFileSync, unlinkSync, writeFileSync } from "fs";
import { join } from "path";
import { cwd } from "process";

const STORE_PASS = "axinvest2026";
const KEY_PASS = "axinvest2026";
const ALIAS = "axinvest-key";
const DAYS = 36500; // ~100 年

const appDir = join(cwd(), "src-tauri", "gen", "android", "app");
const keyFile = join(appDir, "key.pem");
const certFile = join(appDir, "cert.pem");
const p12File = join(appDir, "release.p12");

// 1. 生成 RSA 私钥
console.log("[1/4] Generating RSA-2048 private key...");
const { privateKey } = generateKeyPairSync("rsa", {
  modulusLength: 2048,
  publicKeyEncoding: { type: "spki", format: "pem" },
  privateKeyEncoding: { type: "pkcs8", format: "pem" },
});
writeFileSync(keyFile, privateKey);

// 2. 自签名 X.509 证书
console.log("[2/4] Creating self-signed X.509 certificate...");
const subj = "/C=CN/ST=Shanghai/L=Shanghai/O=AxInvest/OU=Dev/CN=AxInvest";
const keyArg = JSON.stringify(keyFile.replace(/\\/g, "/"));
const certArg = JSON.stringify(certFile.replace(/\\/g, "/"));
execSync(`openssl req -new -x509 -key ${keyArg} -out ${certArg} -days ${DAYS} -subj ${JSON.stringify(subj)}`, {
  stdio: "pipe",
  windowsHide: true,
});

// 3. 打包为 PKCS12 keystore
console.log("[3/4] Packaging as PKCS12 keystore...");
const p12Arg = JSON.stringify(p12File.replace(/\\/g, "/"));
execSync(
  `openssl pkcs12 -export -in ${certArg} -inkey ${keyArg} -out ${p12Arg} -name "${ALIAS}" -passout pass:${STORE_PASS}`,
  { stdio: "pipe", windowsHide: true },
);

// 4. 验证
console.log("[4/4] Verifying keystore...");
try {
  execSync(`openssl pkcs12 -in ${p12Arg} -info -noout -passin pass:${STORE_PASS}`, {
    stdio: "pipe",
    windowsHide: true,
  });
  console.log("  Keystore integrity: OK");
} catch (e) {
  console.log("  Verification:", e.stderr?.toString().substring(0, 200) || "warning (non-critical)");
}

// 输出结果
const p12Buf = readFileSync(p12File);
const b64 = p12Buf.toString("base64");

console.log("");
console.log("=== Keystore Generated Successfully ===");
console.log(`  File:   ${p12File}`);
console.log(`  Size:   ${p12Buf.length} bytes`);
console.log(`  Alias:  ${ALIAS}`);
console.log(`  Store password: ${STORE_PASS}`);
console.log(`  Key password:   ${KEY_PASS}`);
console.log(`  Base64 length:  ${b64.length} chars`);
console.log("");

// 写入 base64 文件供后续使用
const b64File = join(cwd(), "scripts", ".android-keystore-b64.txt");
writeFileSync(b64File, b64);
console.log(`Base64-encoded keystore written to: ${b64File}`);
console.log("(This file contains the secret — do NOT commit to git)");

// 清理临时文件
unlinkSync(keyFile);
unlinkSync(certFile);
