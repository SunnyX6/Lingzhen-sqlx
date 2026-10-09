# Lingzhen-sqlx

基于官方 SQLx 0.9 的 SQL ID 查询扩展。领域初始化时递归加载 SQL 目录，业务代码按 ID 创建原生 SQLx 查询。

## 引入

~~~toml
[dependencies]
sqlx = { package = "lingzhen-sqlx", git = "https://github.com/SunnyX6/Lingzhen-sqlx.git", features = ["sqlite", "runtime-tokio", "derive"] }
~~~

项目中的 sqlx 名称指向此扩展。扩展内部依赖官方 SQLx，并转导出其连接、事务、类型、派生宏和数据库功能。默认不启用数据库驱动或异步运行时。

## SQL 声明

~~~sql
--id id1
INSERT INTO records (
    id,
    title
)
VALUES (
    ?1,
    ?2
);

--id id2
SELECT
    id,
    title
FROM records
WHERE id = ?1;
~~~

一个文件可以声明多个 ID。标记必须独占一行，格式为 --id 加空白及 ID；ID 是后面完整的字符串，只去掉外侧空白，区分大小写，不解释点号、下划线、斜杠或领域含义。

SQL 块在下一个标记或文件末尾结束，不按分号切块。SQL 原文中的换行、空白和普通注释都保留。字符串、带引号标识符、块注释和 PostgreSQL 美元引用中的标记文本不作为声明；引号内支持 SQL 标准的重复引号转义。

一个 ID 对应一次原生 SQLx 查询。只有空白和注释的 SQL 块会在加载时失败。真正的 SQL 语法、参数及字段类型由 SQLx 和数据库在执行时检查。

## 领域初始化

在领域 mod.rs 的现有构造逻辑中调用一次：

~~~rust,ignore
sqlx::load_dir(sql_directory)?;
~~~

sql_directory 是该领域实际可读取的 SQL 根目录。插件递归读取任意层级下的 .sql 文件，构建内部共享索引。重复加载同一个目录不会重新解析文件；不同目录中的 ID 也不能重复。

加载完成后业务查询仅查 HashMap 索引，不读取文件。每次加载先建立待注册索引，全部声明通过后才注册；文件读取、声明或重复 ID 错误不会留下部分注册数据。

发布软件时，调用项目需要将这些 SQL 文件放入自己的资源包，并在领域初始化时传入实际资源目录。扩展不绑定源码目录、安装目录或应用框架。

## 业务调用

~~~rust,ignore
sqlx::query("id1")?
    .bind(1_i64)
    .bind("标题")
    .execute(&pool)
    .await?;

#[derive(sqlx::FromRow)]
struct Record {
    id: i64,
    title: String,
}

let record = sqlx::query_as::<_, Record>("id2")?
    .bind(1_i64)
    .fetch_one(&pool)
    .await?;
~~~

query 和 query_as 返回官方 SQLx 的 Query 和 QueryAs。第一个问号处理 ID 查找错误，最后一个处理数据库执行错误；参数绑定、查询映射、缓存和事务都继续使用 SQLx 原生能力。

## 事务

~~~rust,ignore
let mut transaction = pool.begin().await?;

sqlx::query("id1")?
    .bind(2_i64)
    .bind("事务标题")
    .execute(&mut *transaction)
    .await?;

transaction.commit().await?;
~~~

## 接口和错误

对外新增的入口只有 load_dir、query、query_as。它们返回官方 SQLx 的 Result 和 Error；SQL 声明及 ID 查找错误通过 Error::Configuration 返回，文件读写错误保留原始 IO 错误。

重复 ID 错误包含 ID 和两个声明的文件、行号。缺少 ID、缺少 SQL、SQL 前未声明 ID、未知 ID 分别报告具体原因。未知 ID 不会当作原始 SQL 执行。

官方 SQLx 的其他 API 按原有语义转导出，包括 raw_sql 和宏；它们不解析 SQL ID。本扩展的两个查询函数使用 FromRow 在运行时直接映射结果，不提供查询宏的编译期数据库结构检查。

## 文件职责

- src/lib.rs：SQLx 能力转导出及按 ID 创建原生查询。
- src/loader.rs：目录递归加载、SQL 标记解析、共享索引和错误定位。
- Cargo.toml：官方 SQLx 依赖、功能转发及 Rust 检查规则。

## 验证命令

~~~sh
cargo fmt --check
cargo clippy --features sqlite,runtime-tokio,derive -- -D warnings
cargo check --features sqlite,runtime-tokio,derive
~~~
