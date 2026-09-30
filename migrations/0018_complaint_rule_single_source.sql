-- 投诉定级规则「单一来源」：阈值与口径只在本文件里定义一次。
--
-- 之前阈值散在两处（客户端 Dart 一份、服务端 SQL 一份），改一处忘另一处就会出现
-- "卡片标预警、列表按警告排"的错位。现在收敛为：
--
--   规则函数（数字只在这里出现）
--     complaint_rule_version()          规则版本，如 v2
--     complaint_rule_min_staff_size()   按率定级所需最小规模（低于它退回次数口径）
--     complaint_rule_rate_thresholds()  每百人投诉率上界：0.5 / 1.5 / 3.0（%）
--     complaint_rule_count_thresholds() 次数下界：[0, 1, 3, 6, 10]
--     complaint_rule_levels()           等级序列（由优到劣）
--        │
--        ├─ complaint_rate_percent()      每百人投诉率（规模不可用时 NULL）
--        ├─ complaint_level_rank()        等级序 0..4 —— 排序（ORDER BY）用它
--        ├─ complaint_level()             等级名 excellent..severe —— 响应字段用它
--        └─ complaint_basis()             口径 rate / count —— 响应字段用它
--        │
--        └─ GET /api/v1/complaint-rules   把同一批数字原样返回给客户端渲染规则页
--
-- 于是：客户端不再自己算等级（直接渲染响应里的 complaintLevel / complaintRatePercent /
-- complaintBasis），规则页的区间也从接口取；服务端排序、字段、接口三者同源。

CREATE OR REPLACE FUNCTION complaint_rule_version()
RETURNS TEXT
LANGUAGE sql IMMUTABLE PARALLEL SAFE
AS $$ SELECT 'v2'::text $$;

CREATE OR REPLACE FUNCTION complaint_rule_min_staff_size()
RETURNS INTEGER
LANGUAGE sql IMMUTABLE PARALLEL SAFE
AS $$ SELECT 50 $$;

-- 每百人投诉率的上界（%）：≤0.5 轻微 / ≤1.5 预警 / ≤3.0 警告 / >3.0 严重
CREATE OR REPLACE FUNCTION complaint_rule_rate_thresholds()
RETURNS NUMERIC[]
LANGUAGE sql IMMUTABLE PARALLEL SAFE
AS $$ SELECT ARRAY[0.5, 1.5, 3.0]::numeric[] $$;

-- 投诉次数的下界：[0, 1, 3, 6, 10]（与客户端 complaintLevelThresholds 同形）
CREATE OR REPLACE FUNCTION complaint_rule_count_thresholds()
RETURNS INTEGER[]
LANGUAGE sql IMMUTABLE PARALLEL SAFE
AS $$ SELECT ARRAY[0, 1, 3, 6, 10]::integer[] $$;

-- 等级序列（由优到劣），与客户端 ComplaintLevel 枚举顺序一致
CREATE OR REPLACE FUNCTION complaint_rule_levels()
RETURNS TEXT[]
LANGUAGE sql IMMUTABLE PARALLEL SAFE
AS $$ SELECT ARRAY['excellent', 'minor', 'alert', 'warning', 'severe']::text[] $$;

COMMENT ON FUNCTION complaint_rule_version() IS '投诉定级规则版本（当前 v2：按公司规模折算的每百人投诉率）';
COMMENT ON FUNCTION complaint_rule_min_staff_size() IS '按率定级所需的最小企业规模（人）；低于它退回投诉次数口径，避免小样本波动冤枉小公司';
COMMENT ON FUNCTION complaint_rule_rate_thresholds() IS '每百人投诉率上界（%）：0.5 轻微 / 1.5 预警 / 3.0 警告，超过 3.0 为严重';
COMMENT ON FUNCTION complaint_rule_count_thresholds() IS '投诉次数下界：[0, 1, 3, 6, 10]，即 0 优秀 / 1-2 轻微 / 3-5 预警 / 6-9 警告 / >=10 严重';
COMMENT ON FUNCTION complaint_rule_levels() IS '等级序列（由优到劣）：excellent / minor / alert / warning / severe';

-- 每百人投诉率（%）；规模未知或低于阈值时返回 NULL（= 应退回次数口径）
CREATE OR REPLACE FUNCTION complaint_rate_percent(complaints INTEGER, staff_size INTEGER)
RETURNS NUMERIC
LANGUAGE sql IMMUTABLE PARALLEL SAFE
AS $$
    SELECT CASE
               WHEN staff_size IS NULL
                    OR staff_size < complaint_rule_min_staff_size() THEN NULL
               ELSE COALESCE(complaints, 0)::numeric * 100 / staff_size
           END
$$;

-- 次数口径（阈值来自 complaint_rule_count_thresholds，不再写死）
CREATE OR REPLACE FUNCTION complaint_level_rank(complaints INTEGER)
RETURNS SMALLINT
LANGUAGE sql IMMUTABLE PARALLEL SAFE
AS $$
    SELECT CASE
               WHEN COALESCE(complaints, 0) <= 0 THEN 0::smallint
               WHEN complaints >= (complaint_rule_count_thresholds())[5] THEN 4::smallint  -- >=10 严重
               WHEN complaints >= (complaint_rule_count_thresholds())[4] THEN 3::smallint  -- 6-9 警告
               WHEN complaints >= (complaint_rule_count_thresholds())[3] THEN 2::smallint  -- 3-5 预警
               ELSE 1::smallint                                                           -- 1-2 轻微
           END
$$;

-- 定级 v2：有规模按率、否则退回次数（阈值全部来自上面的规则函数）
CREATE OR REPLACE FUNCTION complaint_level_rank(complaints INTEGER, staff_size INTEGER)
RETURNS SMALLINT
LANGUAGE sql IMMUTABLE PARALLEL SAFE
AS $$
    SELECT CASE
               WHEN COALESCE(complaints, 0) <= 0 THEN 0::smallint   -- 优秀
               WHEN complaint_rate_percent(complaints, staff_size) IS NULL
                   THEN complaint_level_rank(complaints)            -- 未申报/规模过小：次数口径
               WHEN complaint_rate_percent(complaints, staff_size)
                        > (complaint_rule_rate_thresholds())[3] THEN 4::smallint            -- >3.0% 严重
               WHEN complaint_rate_percent(complaints, staff_size)
                        > (complaint_rule_rate_thresholds())[2] THEN 3::smallint            -- >1.5% 警告
               WHEN complaint_rate_percent(complaints, staff_size)
                        > (complaint_rule_rate_thresholds())[1] THEN 2::smallint            -- >0.5% 预警
               ELSE 1::smallint                                                           -- <=0.5% 轻微
           END
$$;

COMMENT ON FUNCTION complaint_level_rank(INTEGER, INTEGER) IS
    '投诉定级 v2（投诉次数, 企业规模）-> 等级序 0..4；规则与阈值见 complaint_rule_* 函数（单一来源）';

-- 等级名（与客户端 ComplaintLevel 枚举名一致），响应字段 complaintLevel 直接用它
CREATE OR REPLACE FUNCTION complaint_level(complaints INTEGER, staff_size INTEGER)
RETURNS TEXT
LANGUAGE sql IMMUTABLE PARALLEL SAFE
AS $$
    SELECT (complaint_rule_levels())[complaint_level_rank(complaints, staff_size) + 1]
$$;

COMMENT ON FUNCTION complaint_level(INTEGER, INTEGER) IS
    '投诉定级 v2 -> 等级名：excellent / minor / alert / warning / severe（客户端直接渲染，不再自行计算）';

-- 口径：rate（按规模折算的投诉率）/ count（次数兜底）
CREATE OR REPLACE FUNCTION complaint_basis(complaints INTEGER, staff_size INTEGER)
RETURNS TEXT
LANGUAGE sql IMMUTABLE PARALLEL SAFE
AS $$
    SELECT CASE
               WHEN complaint_rate_percent(complaints, staff_size) IS NULL THEN 'count'::text
               ELSE 'rate'::text
           END
$$;

COMMENT ON FUNCTION complaint_basis(INTEGER, INTEGER) IS
    '定级所用口径：rate（规模 >= 阈值，按每百人投诉率）/ count（未申报规模或规模过小，按投诉次数）';
