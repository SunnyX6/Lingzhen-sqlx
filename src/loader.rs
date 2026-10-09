use std::collections::{hash_map::Entry, HashMap, HashSet};
use std::fmt;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Arc, LazyLock, RwLock};

use sqlx_native::{AssertSqlSafe, SqlSafeStr, SqlStr};

static REGISTRY: LazyLock<RwLock<Registry>> = LazyLock::new(|| RwLock::new(Registry::default()));

#[derive(Default)]
struct Registry {
    directories: HashSet<PathBuf>,
    statements: HashMap<String, Statement>,
}

struct Statement {
    sql: SqlStr,
    path: PathBuf,
    line: usize,
}

enum SqlContext {
    Normal,
    Quoted(u8),
    BlockComment(usize),
    DollarQuoted(String),
}

#[derive(Debug)]
enum Error {
    MissingId {
        path: PathBuf,
        line: usize,
    },
    UnmarkedSql {
        path: PathBuf,
        line: usize,
    },
    EmptySql {
        id: String,
        path: PathBuf,
        line: usize,
    },
    DuplicateId {
        id: String,
        first_path: PathBuf,
        first_line: usize,
        second_path: PathBuf,
        second_line: usize,
    },
    UnknownId(String),
}

impl fmt::Display for Error {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::MissingId { path, line } => {
                write!(formatter, "{}:{line}：--id 后缺少 ID", path.display())
            }
            Self::UnmarkedSql { path, line } => {
                write!(formatter, "{}:{line}：SQL 前缺少 --id 声明", path.display())
            }
            Self::EmptySql { id, path, line } => {
                write!(formatter, "{}:{line}：ID「{id}」未提供 SQL", path.display())
            }
            Self::DuplicateId {
                id,
                first_path,
                first_line,
                second_path,
                second_line,
            } => write!(
                formatter,
                "SQL ID「{id}」重复：{}:{first_line} 与 {}:{second_line}",
                first_path.display(),
                second_path.display(),
            ),
            Self::UnknownId(id) => write!(formatter, "SQL ID「{id}」未注册"),
        }
    }
}

impl std::error::Error for Error {}

/// 递归加载目录内的 .sql 文件；同一目录只注册一次，失败时不注册部分数据。
pub fn load_dir(directory: impl AsRef<Path>) -> sqlx_native::Result<()> {
    let directory = fs::canonicalize(directory)?;
    let mut registry = REGISTRY.write().unwrap();
    if registry.directories.contains(&directory) {
        return Ok(());
    }

    let mut pending = HashMap::<String, Statement>::new();
    let mut directories = vec![directory.clone()];
    let mut visited = HashSet::new();

    while let Some(path) = directories.pop() {
        if !visited.insert(path.clone()) {
            continue;
        }
        let mut entries = fs::read_dir(&path)?.collect::<std::io::Result<Vec<_>>>()?;
        entries.sort_by_key(|entry| entry.file_name());
        for entry in entries {
            let path = entry.path();
            let metadata = fs::metadata(&path)?;
            if metadata.is_dir() {
                directories.push(fs::canonicalize(&path)?);
                continue;
            }
            if !metadata.is_file()
                || path.extension().and_then(|value| value.to_str()) != Some("sql")
            {
                continue;
            }
            let source = fs::read_to_string(&path)?;
            for (id, statement) in parse(&path, &source)? {
                match pending.entry(id) {
                    Entry::Vacant(entry) => {
                        entry.insert(statement);
                    }
                    Entry::Occupied(entry) => {
                        let first = entry.get();
                        return Err(sqlx_native::Error::Configuration(Box::new(
                            Error::DuplicateId {
                                id: entry.key().clone(),
                                first_path: first.path.clone(),
                                first_line: first.line,
                                second_path: statement.path,
                                second_line: statement.line,
                            },
                        )));
                    }
                }
            }
        }
    }

    for (id, statement) in &pending {
        if let Some(first) = registry.statements.get(id) {
            return Err(sqlx_native::Error::Configuration(Box::new(
                Error::DuplicateId {
                    id: id.clone(),
                    first_path: first.path.clone(),
                    first_line: first.line,
                    second_path: statement.path.clone(),
                    second_line: statement.line,
                },
            )));
        }
    }
    registry.statements.extend(pending);
    registry.directories.insert(directory);
    Ok(())
}

pub(crate) fn get(id: &str) -> sqlx_native::Result<SqlStr> {
    let registry = REGISTRY.read().unwrap();
    registry
        .statements
        .get(id)
        .map(|statement| statement.sql.clone())
        .ok_or_else(|| sqlx_native::Error::Configuration(Box::new(Error::UnknownId(id.to_owned()))))
}

fn parse(path: &Path, source: &str) -> sqlx_native::Result<Vec<(String, Statement)>> {
    let mut statements = Vec::new();
    let mut context = SqlContext::Normal;
    let mut current: Option<(String, usize)> = None;
    let mut sql_start = 0;
    let mut offset = 0;
    let mut has_sql = false;
    let mut first_sql_line = 0;

    for (line_index, line) in source.split_inclusive('\n').enumerate() {
        let line_number = line_index + 1;
        let trimmed = line.trim_start();
        if let SqlContext::Normal = context {
            if let Some(rest) = trimmed.strip_prefix("--id") {
                if rest.is_empty() || rest.starts_with(char::is_whitespace) {
                    let id = rest.trim();
                    if id.is_empty() {
                        return Err(sqlx_native::Error::Configuration(Box::new(
                            Error::MissingId {
                                path: path.to_owned(),
                                line: line_number,
                            },
                        )));
                    }
                    if let Some((previous_id, previous_line)) = current.take() {
                        if !has_sql {
                            return Err(sqlx_native::Error::Configuration(Box::new(
                                Error::EmptySql {
                                    id: previous_id,
                                    path: path.to_owned(),
                                    line: previous_line,
                                },
                            )));
                        }
                        statements.push((
                            previous_id,
                            Statement {
                                sql: AssertSqlSafe(Arc::<str>::from(&source[sql_start..offset]))
                                    .into_sql_str(),
                                path: path.to_owned(),
                                line: previous_line,
                            },
                        ));
                    } else if has_sql {
                        return Err(sqlx_native::Error::Configuration(Box::new(
                            Error::UnmarkedSql {
                                path: path.to_owned(),
                                line: first_sql_line,
                            },
                        )));
                    }
                    current = Some((id.to_owned(), line_number));
                    has_sql = false;
                    offset += line.len();
                    sql_start = offset;
                    continue;
                }
            }
        }

        let bytes = line.as_bytes();
        let mut index = 0;
        while index < bytes.len() {
            match &mut context {
                SqlContext::Normal => {
                    if bytes[index..].starts_with(b"--") {
                        break;
                    }
                    if bytes[index..].starts_with(b"/*") {
                        context = SqlContext::BlockComment(1);
                        index += 2;
                        continue;
                    }
                    let byte = bytes[index];
                    if !byte.is_ascii_whitespace() && byte != b';' {
                        if !has_sql {
                            first_sql_line = line_number;
                        }
                        has_sql = true;
                    }
                    match byte {
                        b'\'' | b'"' | 0x60 => {
                            context = SqlContext::Quoted(byte);
                            index += 1;
                        }
                        b'[' => {
                            context = SqlContext::Quoted(b']');
                            index += 1;
                        }
                        b'$' => {
                            let mut end = index + 1;
                            if end < bytes.len()
                                && (bytes[end] == b'$'
                                    || bytes[end].is_ascii_alphabetic()
                                    || bytes[end] == b'_')
                            {
                                while end < bytes.len()
                                    && (bytes[end].is_ascii_alphanumeric() || bytes[end] == b'_')
                                {
                                    end += 1;
                                }
                                if end < bytes.len() && bytes[end] == b'$' {
                                    context =
                                        SqlContext::DollarQuoted(line[index..=end].to_owned());
                                    index = end + 1;
                                    continue;
                                }
                            }
                            index += 1;
                        }
                        _ => index += 1,
                    }
                }
                SqlContext::Quoted(quote) => {
                    if bytes[index] == *quote {
                        if bytes.get(index + 1) == Some(quote) {
                            index += 2;
                        } else {
                            context = SqlContext::Normal;
                            index += 1;
                        }
                    } else {
                        index += 1;
                    }
                }
                SqlContext::BlockComment(depth) => {
                    if bytes[index..].starts_with(b"/*") {
                        *depth += 1;
                        index += 2;
                    } else if bytes[index..].starts_with(b"*/") {
                        *depth -= 1;
                        if *depth == 0 {
                            context = SqlContext::Normal;
                        }
                        index += 2;
                    } else {
                        index += 1;
                    }
                }
                SqlContext::DollarQuoted(delimiter) => {
                    if bytes[index..].starts_with(delimiter.as_bytes()) {
                        index += delimiter.len();
                        context = SqlContext::Normal;
                    } else {
                        index += 1;
                    }
                }
            }
        }
        offset += line.len();
    }

    if let Some((id, line)) = current {
        if !has_sql {
            return Err(sqlx_native::Error::Configuration(Box::new(
                Error::EmptySql {
                    id,
                    path: path.to_owned(),
                    line,
                },
            )));
        }
        statements.push((
            id,
            Statement {
                sql: AssertSqlSafe(Arc::<str>::from(&source[sql_start..])).into_sql_str(),
                path: path.to_owned(),
                line,
            },
        ));
    } else if has_sql {
        return Err(sqlx_native::Error::Configuration(Box::new(
            Error::UnmarkedSql {
                path: path.to_owned(),
                line: first_sql_line,
            },
        )));
    }
    Ok(statements)
}
