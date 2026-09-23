-- 用人单位投诉等级：投诉次数 -> 等级，职位列表一律按「等级从优到劣」返回。
--
-- 五档阈值与客户端（求职 App 的「投诉等级规则」页 / complaintLevelThresholds）完全一致：
--     0 次          -> 优秀 excellent
--     1 - 2 次      -> 轻微 minor
--     3 - 5 次      -> 预警 alert
--     6 - 9 次      -> 警告 warning
--     10 次及以上   -> 严重 severe
--
-- 口径：只统计管理员审核通过的投诉（见 0012_complaints.sql，审核通过时才 complaints_count + 1），
-- 因此每条计入的都是「有效投诉」，不含随手点击的噪声。
--
-- 说明：等级是 complaints_count 的单调不减函数，所以「按等级升序」目前与「按次数升序」等价；
-- 仍把等级显式做成函数，一是让 SQL 直接表达契约（不必让读代码的人自己在脑子里做等价推导），
-- 二是后续阈值调整、按等级过滤/分组都复用同一个函数，只有这一处需要改。

CREATE FUNCTION complaint_level_rank(complaints INTEGER)
RETURNS SMALLINT
LANGUAGE sql
IMMUTABLE
PARALLEL SAFE
AS $$
    SELECT CASE
               WHEN COALESCE(complaints, 0) <= 0 THEN 0::smallint   -- 优秀
               WHEN complaints <= 2 THEN 1::smallint                -- 轻微
               WHEN complaints <= 5 THEN 2::smallint                -- 预警
               WHEN complaints <= 9 THEN 3::smallint                -- 警告
               ELSE 4::smallint                                     -- 严重
           END
$$;

COMMENT ON FUNCTION complaint_level_rank(INTEGER) IS
    '投诉次数 -> 投诉等级序：0 优秀 / 1 轻微 / 2 预警 / 3 警告 / 4 严重；职位列表按该函数升序（等级从优到劣）返回';

COMMENT ON COLUMN companies.complaints_count IS
    '被投诉次数（>=0，只计管理员审核通过的投诉）；等级见 complaint_level_rank：0 优秀 / 1-2 轻微 / 3-5 预警 / 6-9 警告 / >=10 严重；职位列表按等级从优到劣返回';
