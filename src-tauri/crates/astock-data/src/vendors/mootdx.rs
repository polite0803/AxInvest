use crate::as_of_capability::AsOfCapability;
use crate::error::DataError;
use crate::types::*;
use crate::vendors::StockVendor;
use async_trait::async_trait;
use std::sync::atomic::{AtomicUsize, Ordering};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;

/// 通达信行情服务器候选（2026-10-01 按 pytdx 现行 `hq_hosts` 重整）。
///
/// 原列表只有 7 个且串行 failover ⇒ 最坏耗时 = 7 × 超时。配合 `race_with` 的
/// 并行 racing 后，「列表长度」不再影响最坏耗时，因此这里放开到覆盖各运营商
/// （电信/联通/移动/云行情）—— TDX 服务器按机房与运营商分布，跨网时换一个往往就通
/// （原 7 个里有 6 个今日仍在 pytdx 列表中，说明地址本身没错，是可达性问题）。
const TDX_SERVERS: &[(&str, u16)] = &[
    ("119.147.212.81", 7709),  // 招商证券深圳
    ("221.231.141.60", 7709),  // 华泰证券(南京电信)
    ("101.227.73.20", 7709),   // 华泰证券(上海电信)
    ("101.227.77.254", 7709),  // 华泰证券(上海电信二)
    ("14.215.128.18", 7709),   // 华泰证券(深圳电信)
    ("59.173.18.140", 7709),   // 华泰证券(武汉电信)
    ("218.108.98.244", 7709),  // 杭州华数主站J1
    ("218.108.47.69", 7709),   // 杭州华数主站J2
    ("60.191.117.167", 7709),  // 杭州电信主站J1
    ("115.238.56.198", 7709),  // 杭州电信主站J2
    ("218.75.126.9", 7709),    // 杭州电信主站J3
    ("115.238.90.165", 7709),  // 杭州电信主站J4
    ("124.160.88.183", 7709),  // 杭州联通主站J1
    ("60.12.136.250", 7709),   // 杭州联通主站J2
    ("218.6.170.47", 7709),    // 上证云成都电信一
    ("123.125.108.14", 7709),  // 上证云北京联通一
    ("180.153.18.170", 7709),  // 上海电信主站Z1
    ("180.153.18.171", 7709),  // 上海电信主站Z2
    ("180.153.39.51", 7709),   // 上海电信主站Z3
    ("202.108.253.130", 7709), // 北京联通主站Z1
    ("202.108.253.131", 7709), // 北京联通主站Z2
    ("114.80.63.12", 7709),    // 云行情上海电信Z1
    ("114.80.63.35", 7709),    // 云行情上海电信Z2
    ("14.17.75.71", 7709),     // 深圳电信主站Z1
];

const RSP_HEADER_LEN: usize = 0x10;

pub struct MootdxVendor {
    pub host: String,
    pub port: u16,
    /// 上次握手成功的 `TDX_SERVERS` 下标（**胜者记忆**）。
    /// `race_with` 先直连它（多数调用一次命中、不产生并发连接），只有它失败才
    /// racing 全表，胜出者再写回这里。AtomicUsize 是因为 vendor 方法签名是 `&self`，
    /// 需要内部可变性。
    current_server_idx: AtomicUsize,
}

impl MootdxVendor {
    pub fn new() -> Self {
        let (host, port) = TDX_SERVERS[0];
        Self { host: host.to_string(), port, current_server_idx: AtomicUsize::new(0) }
    }

    async fn connect(&self) -> Result<TdxConnection, DataError> {
        // 单服务器超时 2s。`race_with` 用并行 racing ⇒ 最坏耗时 ≈ **单次**超时，
        // 与服务器列表长度无关；2s 对 TCP 握手（RTT 量级）足够宽裕，再长只会让
        // 「全表不可达」时白等更久。注意这类**慢失败永远触不了降级**（健康窗口是
        // 「30s 内失败 8 次」，而它 30s 只失败 1 次），所以超时必须短 —— 这是
        // racing 之外的第二道保险（见 2026-10-01 日志：每次 klines 白等 ~30s）。
        let stream = tokio::time::timeout(
            std::time::Duration::from_secs(2),
            TcpStream::connect((&*self.host, self.port)),
        )
        .await
        .map_err(|_| {
            DataError::IoError(std::io::Error::new(
                std::io::ErrorKind::TimedOut,
                format!("TDX connect timeout: {}:{}", self.host, self.port),
            ))
        })?
        .map_err(DataError::IoError)?;

        let mut conn = TdxConnection { stream };

        conn.setup().await?;

        Ok(conn)
    }

    /// 构造指向指定服务器的临时 vendor（`connect` 读的是 `self.host` / `self.port`）
    fn at_server(idx: usize, host: &str, port: u16) -> Self {
        Self { host: host.to_string(), port, current_server_idx: AtomicUsize::new(idx) }
    }

    /// 并行 racing + **取数验证**：每个候选连上后立刻执行 `probe`，第一个真正拿到
    /// 数据的才胜出（只握手成功不算）。
    ///
    /// 为什么必须把取数纳入 racing（2026-10-01 运行日志实证）：
    /// ① 串行 failover 的最坏耗时 = 服务器数 × 超时（实测每次 klines 白等 ~30s），
    ///    且这种「慢失败」永远触不了降级（窗口要 30s 内 8 次，它 30s 只失败 1 次）；
    /// ② 更要紧的是 **TDX 服务器能力并不一致** ——「能握手」≠「能给你要的数据」。
    ///    racing 修好连接后，第一个连上的服务器直接返回 `no quote data from TDX server`；
    ///    若只看连接成功就记成胜者，后续会反复复用这台给不出数据的服务器。
    /// 故胜者判据是 `probe` 成功，而非 TCP 握手成功。`probe` 会被每个候选各调用一次，
    /// 因此要求 `Clone`。
    async fn race_with<T, F>(&self, probe: F) -> Result<T, DataError>
    where
        T: Send + 'static,
        F: Fn(TdxConnection) -> futures::future::BoxFuture<'static, Result<T, DataError>>
            + Send
            + Sync
            + Clone
            + 'static,
    {
        type Attempt<T> = futures::future::BoxFuture<'static, Result<(usize, T), DataError>>;

        let preferred = self.current_server_idx.load(Ordering::Relaxed);
        if let Some(&(host, port)) = TDX_SERVERS.get(preferred) {
            // 先试上次的胜者：多数调用一次命中，不产生并发连接
            if let Ok(conn) = Self::at_server(preferred, host, port).connect().await {
                // `if let Ok(v) = r` 与旧写法 `if let Some(v) = r.ok()` 逐情形等价
                //（`Result::ok()` 只是 `Ok(v) => Some(v)`），此处不涉及浮点/NaN 边界，
                // 故按 clippy 建议收敛（`clippy::match_result_ok`）。
                if let Ok(value) = probe(conn).await {
                    return Ok(value);
                }
            }
        }

        let mut attempts: Vec<Attempt<T>> = Vec::with_capacity(TDX_SERVERS.len());
        for (idx, &(host, port)) in TDX_SERVERS.iter().enumerate() {
            if idx == preferred {
                continue; // 刚试过，不必重复
            }
            let probe = probe.clone();
            let fut: Attempt<T> = Box::pin(async move {
                let conn = MootdxVendor::at_server(idx, host, port).connect().await?;
                let value = probe(conn).await?;
                Ok((idx, value))
            });
            attempts.push(fut);
        }
        if attempts.is_empty() {
            return Err(DataError::VendorError {
                vendor: "mootdx".into(),
                message: "TDX 候选服务器为空（列表只剩上次的胜者）".into(),
            });
        }

        match futures::future::select_ok(attempts).await {
            Ok(((idx, value), _slower)) => {
                // 记住胜者，下次优先直连（避免每次调用都并发一轮）
                self.current_server_idx.store(idx, Ordering::Relaxed);
                Ok(value)
            },
            Err(e) => Err(e),
        }
    }

    fn market_code(stock_code: &str) -> u8 {
        if stock_code.starts_with('6') || stock_code.starts_with('9') {
            1
        } else if stock_code.starts_with('8') || stock_code.starts_with('4') {
            2
        } else {
            0
        }
    }

    /// 通达信查询目标 `(市场号, 裸码)`。
    ///
    /// 带显式市场标记的输入（`000001.SH` / `sh000001`）必须**同时**换成裸码：
    /// 整串送进 TDX 查不到任何标的，而市场位按「首位 0」推断会把上证综指当深市股票
    /// （上证综指在 TDX 是市场 1 + `000001`）。
    fn tdx_target(stock_code: &str) -> (u8, &str) {
        match crate::code_form::split_explicit_market(stock_code) {
            Some((bare, ex)) => (ex.tdx_market(), bare),
            None => (Self::market_code(stock_code), stock_code),
        }
    }

    fn kline_category(period: &str) -> u16 {
        match period {
            "5" | "Min5" => 4,
            "15" | "Min15" => 5,
            "30" | "Min30" => 6,
            "60" | "Min60" => 7,
            "daily" | "101" | "Daily" | "8" => 8,
            "weekly" | "102" | "Weekly" | "9" => 9,
            "monthly" | "103" | "Monthly" | "10" => 10,
            _ => 8,
        }
    }
}

impl Default for MootdxVendor {
    fn default() -> Self {
        Self::new()
    }
}

struct TdxConnection {
    stream: TcpStream,
}

impl TdxConnection {
    async fn setup(&mut self) -> Result<(), DataError> {
        let setup1: Vec<u8> =
            vec![0x0c, 0x02, 0x18, 0x93, 0x00, 0x01, 0x03, 0x00, 0x03, 0x00, 0x0d, 0x00, 0x01];
        let setup2: Vec<u8> =
            vec![0x0c, 0x02, 0x18, 0x94, 0x00, 0x01, 0x03, 0x00, 0x03, 0x00, 0x0d, 0x00, 0x02];
        let setup3: Vec<u8> = vec![
            0x0c, 0x03, 0x18, 0x99, 0x00, 0x01, 0x20, 0x00, 0x20, 0x00, 0xdb, 0x0f, 0xd5, 0xd0,
            0xc9, 0xcc, 0xd6, 0xa4, 0xa8, 0xaf, 0x00, 0x00, 0x00, 0x8f, 0xc2, 0x25, 0x40, 0x13,
            0x00, 0x00, 0xd5, 0x00, 0xc9, 0xcc, 0xbd, 0xf0, 0xd7, 0xea, 0x00, 0x00, 0x00, 0x02,
        ];

        self.stream.write_all(&setup1).await?;
        self.read_response_body().await?;

        self.stream.write_all(&setup2).await?;
        self.read_response_body().await?;

        self.stream.write_all(&setup3).await?;
        self.read_response_body().await?;

        Ok(())
    }

    async fn send_and_recv(&mut self, pkg: &[u8]) -> Result<Vec<u8>, DataError> {
        self.stream.write_all(pkg).await?;
        self.read_response_body().await
    }

    async fn read_response_body(&mut self) -> Result<Vec<u8>, DataError> {
        let mut header = [0u8; RSP_HEADER_LEN];
        self.stream.read_exact(&mut header).await?;

        let zip_size = u16::from_le_bytes([header[12], header[13]]) as usize;
        let unzip_size = u16::from_le_bytes([header[14], header[15]]) as usize;

        let mut body = vec![0u8; zip_size];
        self.stream.read_exact(&mut body).await?;

        if zip_size != unzip_size && zip_size > 0 {
            let decompressed = decompress_zlib(&body, unzip_size)?;
            Ok(decompressed)
        } else {
            Ok(body)
        }
    }

    async fn get_security_quotes(
        &mut self,
        stocks: &[(u8, &str)],
    ) -> Result<Vec<QuoteResult>, DataError> {
        let stock_len = stocks.len() as u16;
        // 修复 M-RES-7: 原 `stock_len * 7 + 12` 在 stock_len 较大时
        // (u16 * 7 + 12) 理论上不会溢出（max ~462K），但用 checked_mul
        // 防御性更好，符合安全编码规范。
        let pkgdatalen =
            stock_len.checked_mul(7).and_then(|v| v.checked_add(12)).ok_or_else(|| {
                DataError::VendorError {
                    vendor: "mootdx".into(),
                    message: format!("pkgdatalen 计算溢出 (stock_len={})", stock_len),
                }
            })?;

        let mut pkg = Vec::with_capacity(22 + stocks.len() * 7);
        pkg.extend_from_slice(&(0x10cu16).to_le_bytes());
        pkg.extend_from_slice(&0x02006320u32.to_le_bytes());
        pkg.extend_from_slice(&pkgdatalen.to_le_bytes());
        pkg.extend_from_slice(&pkgdatalen.to_le_bytes());
        pkg.extend_from_slice(&0x5053eu32.to_le_bytes());
        pkg.extend_from_slice(&0u32.to_le_bytes());
        pkg.extend_from_slice(&0u16.to_le_bytes());
        pkg.extend_from_slice(&stock_len.to_le_bytes());

        for (market, code) in stocks {
            pkg.push(*market);
            let code_bytes = code.as_bytes();
            for i in 0..6 {
                pkg.push(if i < code_bytes.len() {
                    code_bytes[i]
                } else {
                    0
                });
            }
        }

        let body = self.send_and_recv(&pkg).await?;
        self.parse_quotes(&body)
    }

    fn parse_quotes(&self, body: &[u8]) -> Result<Vec<QuoteResult>, DataError> {
        if body.len() < 4 {
            return Ok(vec![]);
        }

        let mut pos = 2;
        let num_stock = u16::from_le_bytes([body[pos], body[pos + 1]]) as usize;
        pos += 2;

        let mut results = Vec::with_capacity(num_stock);

        for _ in 0..num_stock {
            if pos + 9 > body.len() {
                break;
            }

            let _market = body[pos];
            pos += 1;

            let code_end = (pos + 6).min(body.len());
            let code =
                String::from_utf8_lossy(&body[pos..code_end]).trim_end_matches('\0').to_string();
            pos += 6;

            let _active1 = u16::from_le_bytes([body[pos], body[pos + 1]]);
            pos += 2;

            let (price, new_pos) = get_price(body, pos)?;
            pos = new_pos;

            let (last_close_diff, new_pos) = get_price(body, pos)?;
            pos = new_pos;

            let (open_diff, new_pos) = get_price(body, pos)?;
            pos = new_pos;

            let (high_diff, new_pos) = get_price(body, pos)?;
            pos = new_pos;

            let (low_diff, new_pos) = get_price(body, pos)?;
            pos = new_pos;

            let (_reversed0, new_pos) = get_price(body, pos)?;
            pos = new_pos;

            let (_reversed1, new_pos) = get_price(body, pos)?;
            pos = new_pos;

            let (vol, new_pos) = get_price(body, pos)?;
            pos = new_pos;

            let (_cur_vol, new_pos) = get_price(body, pos)?;
            pos = new_pos;

            if pos + 4 > body.len() {
                break;
            }
            let amount_raw =
                u32::from_le_bytes([body[pos], body[pos + 1], body[pos + 2], body[pos + 3]]);
            let amount = get_volume(amount_raw);
            pos += 4;

            let (_s_vol, new_pos) = get_price(body, pos)?;
            pos = new_pos;

            let (_b_vol, new_pos) = get_price(body, pos)?;
            pos = new_pos;

            let (_rev2, new_pos) = get_price(body, pos)?;
            pos = new_pos;

            let (_rev3, new_pos) = get_price(body, pos)?;
            pos = new_pos;

            for _ in 0..5 {
                let (_, new_pos) = get_price(body, pos)?;
                pos = new_pos;
                let (_, new_pos) = get_price(body, pos)?;
                pos = new_pos;
                let (_, new_pos) = get_price(body, pos)?;
                pos = new_pos;
                let (_, new_pos) = get_price(body, pos)?;
                pos = new_pos;
            }

            if pos + 2 > body.len() {
                break;
            }
            pos += 2;

            for _ in 0..4 {
                let (_, new_pos) = get_price(body, pos)?;
                pos = new_pos;
            }

            if pos + 4 > body.len() {
                break;
            }
            pos += 4;

            let last_close = cal_price(price, last_close_diff);
            let open = cal_price(price, open_diff);
            let high = cal_price(price, high_diff);
            let low = cal_price(price, low_diff);

            results.push(QuoteResult {
                code,
                price: cal_price(price, 0),
                last_close,
                open,
                high,
                low,
                vol: vol as f64,
                amount,
            });
        }

        Ok(results)
    }

    async fn get_security_bars(
        &mut self,
        category: u16,
        market: u16,
        code: &str,
        start: u16,
        count: u16,
    ) -> Result<Vec<KLineResult>, DataError> {
        let mut pkg = Vec::with_capacity(44);
        pkg.extend_from_slice(&(0x10cu16).to_le_bytes());
        pkg.extend_from_slice(&0x01016408u32.to_le_bytes());
        pkg.extend_from_slice(&0x1cu16.to_le_bytes());
        pkg.extend_from_slice(&0x1cu16.to_le_bytes());
        pkg.extend_from_slice(&0x052du16.to_le_bytes());
        pkg.extend_from_slice(&market.to_le_bytes());

        let code_bytes = code.as_bytes();
        for i in 0..6 {
            pkg.push(if i < code_bytes.len() {
                code_bytes[i]
            } else {
                0
            });
        }

        pkg.extend_from_slice(&category.to_le_bytes());
        pkg.extend_from_slice(&1u16.to_le_bytes());
        pkg.extend_from_slice(&start.to_le_bytes());
        pkg.extend_from_slice(&count.to_le_bytes());
        pkg.extend_from_slice(&0u32.to_le_bytes());
        pkg.extend_from_slice(&0u32.to_le_bytes());
        pkg.extend_from_slice(&0u16.to_le_bytes());

        let body = self.send_and_recv(&pkg).await?;
        self.parse_bars(category, &body)
    }

    fn parse_bars(&self, category: u16, body: &[u8]) -> Result<Vec<KLineResult>, DataError> {
        if body.len() < 2 {
            return Ok(vec![]);
        }

        let ret_count = u16::from_le_bytes([body[0], body[1]]) as usize;
        let mut pos = 2;
        let mut klines = Vec::with_capacity(ret_count);
        let mut pre_diff_base: i64 = 0;

        for _ in 0..ret_count {
            if pos + 4 > body.len() {
                break;
            }

            let (year, month, day, hour, minute) = parse_datetime(category, body, pos);
            pos += 4;

            let (open_diff, new_pos) = get_price(body, pos)?;
            pos = new_pos;

            let (close_diff, new_pos) = get_price(body, pos)?;
            pos = new_pos;

            let (high_diff, new_pos) = get_price(body, pos)?;
            pos = new_pos;

            let (low_diff, new_pos) = get_price(body, pos)?;
            pos = new_pos;

            if pos + 8 > body.len() {
                break;
            }

            let vol_raw =
                u32::from_le_bytes([body[pos], body[pos + 1], body[pos + 2], body[pos + 3]]);
            let vol = get_volume(vol_raw);
            pos += 4;

            let amount_raw =
                u32::from_le_bytes([body[pos], body[pos + 1], body[pos + 2], body[pos + 3]]);
            let amount = get_volume(amount_raw);
            pos += 4;

            let open = cal_price1000(open_diff, pre_diff_base);
            let actual_open = open_diff + pre_diff_base;
            let close = cal_price1000(actual_open, close_diff);
            let high = cal_price1000(actual_open, high_diff);
            let low = cal_price1000(actual_open, low_diff);
            pre_diff_base = actual_open + close_diff;

            let date = if category < 4 || category == 7 || category == 8 {
                format!("{year:04}-{month:02}-{day:02}")
            } else {
                format!("{year:04}-{month:02}-{day:02}")
            };

            klines.push(KLineResult {
                date,
                open,
                close,
                high,
                low,
                vol,
                amount,
                _hour: hour,
                _minute: minute,
            });
        }

        Ok(klines)
    }
}

struct QuoteResult {
    code: String,
    price: f64,
    last_close: f64,
    open: f64,
    high: f64,
    low: f64,
    vol: f64,
    amount: f64,
}

struct KLineResult {
    date: String,
    open: f64,
    close: f64,
    high: f64,
    low: f64,
    vol: f64,
    amount: f64,
    _hour: u32,
    _minute: u32,
}

fn get_price(data: &[u8], mut pos: usize) -> Result<(i64, usize), DataError> {
    if pos >= data.len() {
        return Err(DataError::ParseError("get_price out of bounds".into()));
    }

    let mut pos_byte: usize = 6;
    let bdata = data[pos];
    let mut intdata = (bdata & 0x3f) as i64;
    let sign = (bdata & 0x40) != 0;

    if (bdata & 0x80) != 0 {
        loop {
            pos += 1;
            if pos >= data.len() {
                return Err(DataError::ParseError("get_price continuation out of bounds".into()));
            }
            let b = data[pos];
            intdata += ((b & 0x7f) as i64) << pos_byte;
            pos_byte += 7;
            if (b & 0x80) == 0 {
                break;
            }
        }
    }

    pos += 1;

    if sign {
        intdata = -intdata;
    }

    Ok((intdata, pos))
}

fn get_volume(ivol: u32) -> f64 {
    let _logpoint = (ivol >> 24) as i32;
    let hheax = (ivol >> 24) as i32;
    let hleax = ((ivol >> 16) & 0xff) as i32;
    let lheax = ((ivol >> 8) & 0xff) as i32;
    let lleax = (ivol & 0xff) as i32;

    let dw_ecx = hheax * 2 - 0x7f;
    let dw_edx = hheax * 2 - 0x86;
    let dw_esi = hheax * 2 - 0x8e;
    let dw_eax = hheax * 2 - 0x96;

    let dbl_xmm6 = if dw_ecx >= 0 {
        2f64.powi(dw_ecx)
    } else {
        1.0 / 2f64.powi(-dw_ecx)
    };

    let dbl_xmm4 = if hleax > 0x80 {
        let tmp1 = 2f64.powi(dw_edx + 1);
        let dbl_xmm0 = 2f64.powi(dw_edx) * 128.0 + (hleax & 0x7f) as f64 * tmp1;
        dbl_xmm0
    } else if dw_edx >= 0 {
        2f64.powi(dw_edx) * hleax as f64
    } else {
        (1.0 / 2f64.powi(-dw_edx)) * hleax as f64
    };

    let mut dbl_xmm3 = 2f64.powi(dw_esi) * lheax as f64;
    let mut dbl_xmm1 = 2f64.powi(dw_eax) * lleax as f64;

    if hleax & 0x80 != 0 {
        dbl_xmm3 *= 2.0;
        dbl_xmm1 *= 2.0;
    }

    dbl_xmm6 + dbl_xmm4 + dbl_xmm3 + dbl_xmm1
}

fn cal_price(base_p: i64, diff: i64) -> f64 {
    (base_p + diff) as f64 / 100.0
}

fn cal_price1000(base_p: i64, diff: i64) -> f64 {
    (base_p + diff) as f64 / 1000.0
}

fn parse_datetime(category: u16, buffer: &[u8], pos: usize) -> (u32, u32, u32, u32, u32) {
    if pos + 4 > buffer.len() {
        return (0, 0, 0, 15, 0);
    }

    if category < 4 || category == 7 || category == 8 {
        let zipday = u16::from_le_bytes([buffer[pos], buffer[pos + 1]]);
        let tminutes = u16::from_le_bytes([buffer[pos + 2], buffer[pos + 3]]);
        let year = ((zipday as u32) >> 11) + 2004;
        let month = ((zipday as u32) % 2048) / 100;
        let day = (zipday as u32) % 2048 % 100;
        let hour = (tminutes as u32) / 60;
        let minute = (tminutes as u32) % 60;
        (year, month, day, hour, minute)
    } else {
        let zipday =
            u32::from_le_bytes([buffer[pos], buffer[pos + 1], buffer[pos + 2], buffer[pos + 3]]);
        let year = zipday / 10000;
        let month = (zipday % 10000) / 100;
        let day = zipday % 100;
        (year, month, day, 15, 0)
    }
}

fn decompress_zlib(data: &[u8], expected_size: usize) -> Result<Vec<u8>, DataError> {
    use std::io::Read;

    let mut decoder = flate2::read::ZlibDecoder::new(data);
    let mut result = Vec::with_capacity(expected_size);
    decoder
        .read_to_end(&mut result)
        .map_err(|e| DataError::ParseError(format!("zlib decompress failed: {e}")))?;
    Ok(result)
}

#[async_trait]
impl StockVendor for MootdxVendor {
    async fn get_quote(&self, stock_code: &str) -> Result<StockQuote, DataError> {
        let (market, code) = Self::tdx_target(stock_code);
        // 服务器选择与**取数验证**一并交给 `race_with`：不同 TDX 服务器的能力并不一致，
        // 「能握手」不等于「能给行情」（2026-10-01 日志：连上后直接 no quote data）。
        // code 需 owned 才能在 'static 的 racing future 里被多个候选复用，
        // 故先转 String、每次 probe 各 clone 一份。
        let code = code.to_string();
        let quotes = self
            .race_with(move |mut conn| {
                let code = code.clone();
                Box::pin(async move {
                    let stocks = vec![(market, code.as_str())];
                    let quotes = conn.get_security_quotes(&stocks).await?;
                    if quotes.is_empty() {
                        return Err(DataError::VendorError {
                            vendor: "mootdx".into(),
                            message: "no quote data from TDX server".into(),
                        });
                    }
                    Ok(quotes)
                })
            })
            .await?;
        let q = &quotes[0];
        let change_pct = if q.last_close > 0.0 {
            (q.price - q.last_close) / q.last_close * 100.0
        } else {
            0.0
        };
        Ok(StockQuote {
            code: q.code.clone(),
            name: String::new(),
            price: q.price,
            pre_close: q.last_close,
            open: q.open,
            high: q.high,
            low: q.low,
            volume: q.vol * 100.0, // 通达信行情 vol 单位为"手"，×100 转为"股"
            amount: q.amount,
            change_pct,
            turnover_rate: 0.0,
            pe: None,
            pb: None,
            total_mv: None,
            circulating_mv: None,
            limit_up: None,
            limit_down: None,
            is_st: false,
            timestamp: chrono::Utc::now().format("%Y-%m-%d %H:%M:%S").to_string(),
        })
    }

    async fn get_klines(
        &self,
        stock_code: &str,
        period: &str,
        limit: u32,
        _adj: Option<AdjType>,
    ) -> Result<Vec<KLine>, DataError> {
        let (market_u8, code) = Self::tdx_target(stock_code);
        let market = market_u8 as u16;
        let category = Self::kline_category(period);
        // 与 get_quote 同一收口：服务器选择 + 取数验证都在 `race_with` 内完成
        // （见其文档：不同服务器能力不一致，「能握手」不等于「能给你这个周期的 K 线」）。
        let code = code.to_string();
        let bars = self
            .race_with(move |mut conn| {
                let code = code.clone();
                Box::pin(async move {
                    let bars = conn
                        .get_security_bars(category, market, code.as_str(), 0, limit as u16)
                        .await?;
                    if bars.is_empty() {
                        return Err(DataError::VendorError {
                            vendor: "mootdx".into(),
                            message: "no kline data from TDX server".into(),
                        });
                    }
                    Ok(bars)
                })
            })
            .await?;
        Ok(bars
            .into_iter()
            .map(|b| KLine {
                date: b.date,
                open: b.open,
                high: b.high,
                low: b.low,
                close: b.close,
                volume: b.vol * 100.0, // 通达信 K线 vol 单位为"手"，×100 转为"股"
                amount: b.amount,
                turnover_rate: None,
                // P1-4: vendor 默认不复权
                adj_factor: None,
            })
            .collect())
    }

    async fn get_financials(&self, _: &str) -> Result<Vec<FinancialReport>, DataError> {
        Ok(vec![])
    }

    async fn get_news(&self, _: &str, _: u32) -> Result<Vec<NewsItem>, DataError> {
        Ok(vec![])
    }

    async fn get_money_flow(&self, _: &str) -> Result<Option<MoneyFlow>, DataError> {
        Ok(None)
    }

    async fn get_dragon_tiger(&self, _: &str) -> Result<Vec<DragonTigerEntry>, DataError> {
        Ok(vec![])
    }

    async fn get_lockup_schedule(&self, _: &str) -> Result<Vec<LockupSchedule>, DataError> {
        Ok(vec![])
    }

    async fn search_stock(&self, _: &str) -> Result<Vec<StockSearchResult>, DataError> {
        Ok(vec![])
    }

    // ── P3:mootdx 能力申报 ──
    // get_quote:实时TCP行情 → SynthesizeFromKline
    // get_klines:TCP协议,无日期参数 → Fallthrough(lib.rs按date字段截断)
    // 其他 stub:Fallthrough
    fn asof_capability(&self, method: &str) -> AsOfCapability {
        match method {
            "get_quote" => AsOfCapability::SynthesizeFromKline,
            _ => AsOfCapability::Fallthrough,
        }
    }
}

#[cfg(test)]
mod capability_tests {
    use super::*;

    fn make_vendor() -> MootdxVendor {
        MootdxVendor::default()
    }

    #[test]
    fn mootdx_quote_is_synthesize() {
        let v = make_vendor();
        assert_eq!(v.asof_capability("get_quote"), AsOfCapability::SynthesizeFromKline);
    }

    #[test]
    fn mootdx_others_are_fallthrough() {
        let v = make_vendor();
        for m in &[
            "get_klines",
            "get_financials",
            "get_news",
            "get_money_flow",
            "get_dragon_tiger",
            "get_lockup_schedule",
            "search_stock",
        ] {
            assert_eq!(v.asof_capability(m), AsOfCapability::Fallthrough);
        }
    }

    /// TDX 查询目标必须「市场位 + 裸码」一起换：带显式标记的输入若只改市场位、
    /// 把 `000001.SH` 整串送进 TDX，会查不到标的（表现为整源空转）；只剥标记不改
    /// 市场位则把上证综指当深市股票问。
    #[test]
    fn tdx_target_pairs_market_with_bare_code() {
        assert_eq!(MootdxVendor::tdx_target("000001.SH"), (1, "000001"));
        assert_eq!(MootdxVendor::tdx_target("sh000001"), (1, "000001"));
        assert_eq!(MootdxVendor::tdx_target("399006.SZ"), (0, "399006"));
        assert_eq!(MootdxVendor::tdx_target("430047.BJ"), (2, "430047"));
        // 裸码口径不变
        assert_eq!(MootdxVendor::tdx_target("600519"), (1, "600519"));
        assert_eq!(MootdxVendor::tdx_target("000001"), (0, "000001"));
    }
}
