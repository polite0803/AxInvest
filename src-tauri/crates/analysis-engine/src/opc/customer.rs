// SPDX-License-Identifier: AGPL-3.0-only

//! 客户管理领域 — DTO 定义、trait 接口与 SeaORM 实现

use async_trait::async_trait;
use sea_orm::QuerySelect;
use sea_orm::{
    ActiveModelTrait, ColumnTrait, DatabaseConnection, EntityTrait, QueryFilter, QueryOrder, Set,
};
use serde::{Deserialize, Serialize};
use std::str::FromStr;

use super::error::{OpcError, OpcResult};

// ── DTO 定义 ──────────────────────────────────────────────────

/// 客户来源
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum CustomerSource {
    Referral,
    Website,
    SocialMedia,
    Marketplace,
    Direct,
    Other(String),
}

/// 客户类型（B 端 / C 端）。
/// `Unknown` 仅作存量/兜底解析；新增与更新强制二选一。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum CustomerType {
    Consumer,
    Business,
    Unknown,
}

impl CustomerType {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Consumer => "consumer",
            Self::Business => "business",
            Self::Unknown => "unknown",
        }
    }
}

impl std::str::FromStr for CustomerType {
    type Err = String;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "consumer" => Ok(Self::Consumer),
            "business" => Ok(Self::Business),
            _ => Ok(Self::Unknown), // 兜底：存量或未知值不报错
        }
    }
}

/// 客户 DTO
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Customer {
    pub id: String,
    pub name: String,
    pub email: String,
    pub phone: Option<String>,
    pub company: Option<String>,
    pub customer_type: CustomerType,
    pub country: Option<String>,
    pub region: Option<String>,
    pub city: Option<String>,
    pub address: Option<String>,
    pub latitude: Option<f64>,
    pub longitude: Option<f64>,
    pub source: Option<CustomerSource>,
    pub tags: Vec<String>,
    pub notes: String,
    pub total_revenue: f64,
    pub invoice_count: u32,
    pub status: CustomerStatus,
    pub created_at: i64,
    pub updated_at: i64,
}

/// 客户状态
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum CustomerStatus {
    Lead,
    Prospect,
    Active,
    Inactive,
    Churned,
}

impl CustomerStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Lead => "lead",
            Self::Prospect => "prospect",
            Self::Active => "active",
            Self::Inactive => "inactive",
            Self::Churned => "churned",
        }
    }
}

impl std::str::FromStr for CustomerStatus {
    type Err = String;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "lead" => Ok(Self::Lead),
            "prospect" => Ok(Self::Prospect),
            "active" => Ok(Self::Active),
            "inactive" => Ok(Self::Inactive),
            "churned" => Ok(Self::Churned),
            _ => Err(format!("Unknown CustomerStatus: {s}")),
        }
    }
}

/// 创建客户请求
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CreateCustomerInput {
    pub name: String,
    pub email: String,
    pub phone: Option<String>,
    pub company: Option<String>,
    pub customer_type: CustomerType,
    pub country: Option<String>,
    pub region: Option<String>,
    pub city: Option<String>,
    pub address: Option<String>,
    pub latitude: Option<f64>,
    pub longitude: Option<f64>,
    pub source: Option<CustomerSource>,
    pub tags: Vec<String>,
    pub notes: String,
}

/// 更新客户请求
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UpdateCustomerInput {
    pub name: Option<String>,
    pub email: Option<String>,
    pub phone: Option<Option<String>>,
    pub company: Option<Option<String>>,
    pub customer_type: Option<CustomerType>,
    pub country: Option<Option<String>>,
    pub region: Option<Option<String>>,
    pub city: Option<Option<String>>,
    pub address: Option<Option<String>>,
    pub latitude: Option<Option<f64>>,
    pub longitude: Option<Option<f64>>,
    pub tags: Option<Vec<String>>,
    pub notes: Option<String>,
}

/// 客户查询过滤
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct CustomerFilter {
    pub status: Option<CustomerStatus>,
    pub customer_type: Option<CustomerType>,
    pub search: Option<String>,
    pub tags: Option<Vec<String>>,
    pub limit: Option<u32>,
    pub offset: Option<u32>,
}

// ── Customer Service Trait ────────────────────────────────────────

#[async_trait]
pub trait CustomerService: Send + Sync {
    async fn create_customer(&self, input: CreateCustomerInput) -> OpcResult<Customer>;
    async fn get_customer(&self, id: &str) -> OpcResult<Customer>;
    async fn list_customers(&self, filter: CustomerFilter) -> OpcResult<Vec<Customer>>;
    async fn update_customer(&self, id: &str, input: UpdateCustomerInput) -> OpcResult<Customer>;
    async fn delete_customer(&self, id: &str) -> OpcResult<()>;
    async fn find_by_email(&self, email: &str) -> OpcResult<Option<Customer>>;
}

/// Noop 实现
#[derive(Debug)]
pub struct NoopCustomerService;

#[async_trait]
impl CustomerService for NoopCustomerService {
    async fn create_customer(&self, _input: CreateCustomerInput) -> OpcResult<Customer> {
        Err(OpcError::NotFound("CustomerService not implemented".into()))
    }
    async fn get_customer(&self, _id: &str) -> OpcResult<Customer> {
        Err(OpcError::NotFound("CustomerService not implemented".into()))
    }
    async fn list_customers(&self, _filter: CustomerFilter) -> OpcResult<Vec<Customer>> {
        Ok(Vec::new())
    }
    async fn update_customer(&self, _id: &str, _input: UpdateCustomerInput) -> OpcResult<Customer> {
        Err(OpcError::NotFound("CustomerService not implemented".into()))
    }
    async fn delete_customer(&self, _id: &str) -> OpcResult<()> {
        Err(OpcError::NotFound("CustomerService not implemented".into()))
    }
    async fn find_by_email(&self, _email: &str) -> OpcResult<Option<Customer>> {
        Ok(None)
    }
}

// ── SeaORM 实现 ───────────────────────────────────────────────────

use axagent_entities::opc_customers;
use axagent_harness::util_fns::{gen_id, now_ts};

/// 默认客户服务实现
pub struct DefaultCustomerService {
    pub db: DatabaseConnection,
}

impl DefaultCustomerService {
    pub fn new(db: DatabaseConnection) -> Self {
        Self { db }
    }

    /// 存量回填客户类型（幂等）：
    /// 所有未分类（`unknown`）的客户，按 `company` 是否非空归类为 business / consumer。
    /// 存量库在引入 `customer_type` 列之前的数据即为 `unknown`，故此处一次性回填。
    pub async fn backfill_customer_types(&self) -> OpcResult<u64> {
        // 先读出所有未知类型客户，避免一次 UPDATE 扫描全表
        let rows = opc_customers::Entity::find()
            .filter(
                sea_orm::Condition::any()
                    .add(opc_customers::Column::CustomerType.is_null())
                    .add(opc_customers::Column::CustomerType.eq("unknown")),
            )
            .all(&self.db)
            .await
            .map_err(|e| OpcError::Database(e.to_string()))?;

        let now = now_ts();
        let mut updated = 0u64;
        for row in rows {
            let kind = if row.company.as_deref().map(|s| !s.trim().is_empty()).unwrap_or(false) {
                "business"
            } else {
                "consumer"
            };
            let mut am: opc_customers::ActiveModel = row.into();
            am.customer_type = Set(kind.to_string());
            am.updated_at = Set(now);
            am.update(&self.db).await.map_err(|e| OpcError::Database(e.to_string()))?;
            updated += 1;
        }
        Ok(updated)
    }
}

fn entity_to_dto(e: opc_customers::Model) -> OpcResult<Customer> {
    let tags: Vec<String> = serde_json::from_str(&e.tags_json).unwrap_or_default();
    let source = e.source.as_deref().map(|s| match s {
        "referral" => CustomerSource::Referral,
        "website" => CustomerSource::Website,
        "social_media" => CustomerSource::SocialMedia,
        "marketplace" => CustomerSource::Marketplace,
        "direct" => CustomerSource::Direct,
        other => CustomerSource::Other(other.to_string()),
    });
    let status = CustomerStatus::from_str(&e.status).unwrap_or(CustomerStatus::Lead);
    let customer_type = CustomerType::from_str(&e.customer_type).unwrap_or(CustomerType::Unknown);

    Ok(Customer {
        id: e.id,
        name: e.name,
        email: e.email,
        phone: e.phone,
        company: e.company,
        customer_type,
        country: e.country,
        region: e.region,
        city: e.city,
        address: e.address,
        latitude: e.latitude,
        longitude: e.longitude,
        source,
        tags,
        notes: e.notes,
        total_revenue: e.total_revenue,
        invoice_count: e.invoice_count,
        status,
        created_at: e.created_at,
        updated_at: e.updated_at,
    })
}

fn source_to_str(s: &CustomerSource) -> String {
    match s {
        CustomerSource::Referral => "referral".into(),
        CustomerSource::Website => "website".into(),
        CustomerSource::SocialMedia => "social_media".into(),
        CustomerSource::Marketplace => "marketplace".into(),
        CustomerSource::Direct => "direct".into(),
        CustomerSource::Other(v) => v.clone(),
    }
}

#[async_trait]
impl CustomerService for DefaultCustomerService {
    async fn create_customer(&self, input: CreateCustomerInput) -> OpcResult<Customer> {
        let id = gen_id();
        let now = now_ts();

        opc_customers::ActiveModel {
            id: Set(id.clone()),
            name: Set(input.name),
            email: Set(input.email),
            phone: Set(input.phone),
            company: Set(input.company),
            customer_type: Set(input.customer_type.as_str().to_string()),
            country: Set(input.country),
            region: Set(input.region),
            city: Set(input.city),
            address: Set(input.address),
            latitude: Set(input.latitude),
            longitude: Set(input.longitude),
            source: Set(input.source.as_ref().map(source_to_str)),
            tags_json: Set(serde_json::to_string(&input.tags).unwrap_or_else(|_| "[]".into())),
            notes: Set(input.notes),
            total_revenue: Set(0.0),
            invoice_count: Set(0),
            status: Set(CustomerStatus::Lead.as_str().to_string()),
            created_at: Set(now),
            updated_at: Set(now),
        }
        .insert(&self.db)
        .await
        .map_err(|e| OpcError::Database(e.to_string()))?;

        self.get_customer(&id).await
    }

    async fn get_customer(&self, id: &str) -> OpcResult<Customer> {
        let entity = opc_customers::Entity::find_by_id(id)
            .one(&self.db)
            .await
            .map_err(|e| OpcError::Database(e.to_string()))?
            .ok_or_else(|| OpcError::NotFound(format!("customer {id}")))?;

        entity_to_dto(entity)
    }

    async fn list_customers(&self, filter: CustomerFilter) -> OpcResult<Vec<Customer>> {
        let mut query =
            opc_customers::Entity::find().order_by_desc(opc_customers::Column::CreatedAt);

        if let Some(status) = &filter.status {
            query = query.filter(opc_customers::Column::Status.eq(status.as_str()));
        }
        if let Some(customer_type) = &filter.customer_type {
            query = query.filter(opc_customers::Column::CustomerType.eq(customer_type.as_str()));
        }
        if let Some(search) = &filter.search {
            query = query.filter(
                sea_orm::Condition::any()
                    .add(opc_customers::Column::Name.contains(search))
                    .add(opc_customers::Column::Email.contains(search))
                    .add(opc_customers::Column::Company.contains(search)),
            );
        }
        if let Some(limit) = filter.limit {
            query = query.limit(limit as u64);
        }
        if let Some(offset) = filter.offset {
            query = query.offset(offset as u64);
        }

        let entities = query.all(&self.db).await.map_err(|e| OpcError::Database(e.to_string()))?;
        entities.into_iter().map(entity_to_dto).collect()
    }

    async fn update_customer(&self, id: &str, input: UpdateCustomerInput) -> OpcResult<Customer> {
        let entity = opc_customers::Entity::find_by_id(id)
            .one(&self.db)
            .await
            .map_err(|e| OpcError::Database(e.to_string()))?
            .ok_or_else(|| OpcError::NotFound(format!("customer {id}")))?;

        let mut am: opc_customers::ActiveModel = entity.into();
        am.updated_at = Set(now_ts());

        if let Some(name) = input.name {
            am.name = Set(name);
        }
        if let Some(email) = input.email {
            am.email = Set(email);
        }
        if let Some(phone) = input.phone {
            am.phone = Set(phone);
        }
        if let Some(company) = input.company {
            am.company = Set(company);
        }
        if let Some(customer_type) = input.customer_type {
            am.customer_type = Set(customer_type.as_str().to_string());
        }
        if let Some(country) = input.country {
            am.country = Set(country);
        }
        if let Some(region) = input.region {
            am.region = Set(region);
        }
        if let Some(city) = input.city {
            am.city = Set(city);
        }
        if let Some(address) = input.address {
            am.address = Set(address);
        }
        if let Some(latitude) = input.latitude {
            am.latitude = Set(latitude);
        }
        if let Some(longitude) = input.longitude {
            am.longitude = Set(longitude);
        }
        if let Some(tags) = input.tags {
            am.tags_json = Set(serde_json::to_string(&tags).unwrap_or_else(|_| "[]".into()));
        }
        if let Some(notes) = input.notes {
            am.notes = Set(notes);
        }

        am.update(&self.db).await.map_err(|e| OpcError::Database(e.to_string()))?;
        self.get_customer(id).await
    }

    async fn delete_customer(&self, id: &str) -> OpcResult<()> {
        let result = opc_customers::Entity::delete_by_id(id)
            .exec(&self.db)
            .await
            .map_err(|e| OpcError::Database(e.to_string()))?;

        if result.rows_affected == 0 {
            return Err(OpcError::NotFound(format!("customer {id}")));
        }
        Ok(())
    }

    async fn find_by_email(&self, email: &str) -> OpcResult<Option<Customer>> {
        let entity = opc_customers::Entity::find()
            .filter(opc_customers::Column::Email.eq(email))
            .one(&self.db)
            .await
            .map_err(|e| OpcError::Database(e.to_string()))?;

        match entity {
            Some(e) => entity_to_dto(e).map(Some),
            None => Ok(None),
        }
    }
}
