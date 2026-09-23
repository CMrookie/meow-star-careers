//! 通用分页工具

use serde::Deserialize;

/// 分页查询参数
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PageQuery {
    pub page: Option<i64>,
    pub page_size: Option<i64>,
}

/// 归一化分页：返回 (page, page_size, offset)；page>=1，1<=page_size<=100
pub fn paginate(page: Option<i64>, page_size: Option<i64>) -> (i64, i64, i64) {
    let page = page.unwrap_or(1).max(1);
    let page_size = page_size.unwrap_or(20).clamp(1, 100);
    let offset = (page - 1) * page_size;
    (page, page_size, offset)
}
