//! 通过 SQL ID 创建官方 SQLx 查询；目录加载与 SQL 索引由扩展内部管理。

mod loader;

pub use loader::load_dir;
pub use sqlx_native::*;

/// 根据已注册的 SQL ID 创建原生查询，参数绑定与执行继续由 SQLx 处理。
pub fn query<'q, DB>(
    id: &str,
) -> sqlx_native::Result<sqlx_native::query::Query<'q, DB, DB::Arguments>>
where
    DB: sqlx_native::Database,
{
    Ok(sqlx_native::query(loader::get(id)?))
}

/// 根据已注册的 SQL ID 创建直接映射到 Model 或 DTO 的原生查询。
pub fn query_as<'q, DB, O>(
    id: &str,
) -> sqlx_native::Result<sqlx_native::query::QueryAs<'q, DB, O, DB::Arguments>>
where
    DB: sqlx_native::Database,
    O: for<'r> sqlx_native::FromRow<'r, DB::Row>,
{
    Ok(sqlx_native::query_as(loader::get(id)?))
}
