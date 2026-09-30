-- 投诉定级 v2：从「投诉次数」改为「按公司规模折算的每百人投诉率」（小样本退回次数）。
--
-- 背景：固定次数对规模大的公司很不友好 —— 2000 人公司 20 起投诉（1.0%）与
-- 60 人公司 20 起投诉（33%）性质完全不同，按次数会被同等对待。
-- 客户端（求职 App lib/ui/complaint_level.dart，规则版本 v2）已按率定级；
-- 服务端必须用**同一口径**排序，否则翻页时服务端顺序与卡片上标的等级会打架。
--
-- 口径（与客户端逐条对齐；改这里必须同步改客户端与本文件的时间戳说明）：
--   1) 分子只计管理员审核通过的投诉（companies.complaints_count）；
--   2) 分子为 0 -> 优秀；
--   3) 企业已申报规模且 staff_size >= 50 -> 按每百人投诉率（%）：
--        率 <= 0.5  轻微
--        率 <= 1.5  预警
--        率 <= 3.0  警告
--        率 >  3.0  严重
--   4) 未申报规模、或规模 < 50（小样本波动极大，1 起投诉就能把率抬到 2%+，
--      容易冤枉小公司）-> 退回投诉次数口径：1-2 轻微 / 3-5 预警 / 6-9 警告 / >=10 严重。
--
-- 客户端对应常量：minStaffSizeForRate = 50、complaintRateThresholds = [0.5, 1.5, 3.0]。

ALTER TABLE companies ADD COLUMN staff_size INTEGER;

ALTER TABLE companies ADD CONSTRAINT chk_companies_staff_size
    CHECK (staff_size IS NULL OR staff_size >= 0);

-- 次数不允许为负（此前无约束；负值会让率与排序失去意义）
ALTER TABLE companies ADD CONSTRAINT chk_companies_complaints_count
    CHECK (complaints_count >= 0);

COMMENT ON COLUMN companies.staff_size IS
    '企业规模（员工人数，企业申报；NULL = 未申报）。投诉定级 v2 的分母：>=50 人时按每百人投诉率定级，否则退回投诉次数口径';

-- 每百人投诉率（%）；规模未知或 < 50 人时返回 NULL（= 应退回次数口径）
CREATE OR REPLACE FUNCTION complaint_rate_percent(complaints INTEGER, staff_size INTEGER)
RETURNS NUMERIC
LANGUAGE sql
IMMUTABLE
PARALLEL SAFE
AS $$
    SELECT CASE
               WHEN staff_size IS NULL OR staff_size < 50 THEN NULL
               ELSE COALESCE(complaints, 0)::numeric * 100 / staff_size
           END
$$;

COMMENT ON FUNCTION complaint_rate_percent(INTEGER, INTEGER) IS
    '每百人投诉率（%）；规模未知或 < 50 人时返回 NULL，表示应退回投诉次数口径（与客户端 assessComplaints 一致）';

-- 定级 v2：有规模按率、否则退回次数（复用既有的 1 参数次数口径函数）
CREATE OR REPLACE FUNCTION complaint_level_rank(complaints INTEGER, staff_size INTEGER)
RETURNS SMALLINT
LANGUAGE sql
IMMUTABLE
PARALLEL SAFE
AS $$
    SELECT CASE
               WHEN COALESCE(complaints, 0) <= 0 THEN 0::smallint   -- 优秀
               WHEN complaint_rate_percent(complaints, staff_size) IS NULL
                   THEN complaint_level_rank(complaints)            -- 未申报/规模过小：次数口径
               WHEN complaint_rate_percent(complaints, staff_size) <= 0.5 THEN 1::smallint  -- 轻微
               WHEN complaint_rate_percent(complaints, staff_size) <= 1.5 THEN 2::smallint  -- 预警
               WHEN complaint_rate_percent(complaints, staff_size) <= 3.0 THEN 3::smallint  -- 警告
               ELSE 4::smallint                                                             -- 严重
           END
$$;

COMMENT ON FUNCTION complaint_level_rank(INTEGER, INTEGER) IS
    '投诉定级 v2（投诉次数, 企业规模）-> 等级序：0 优秀 / 1 轻微 / 2 预警 / 3 警告 / 4 严重；规模 >=50 人按每百人投诉率（0.5/1.5/3.0%），否则退回次数口径 0/1/2/3/4';

COMMENT ON COLUMN companies.complaints_count IS
    '被投诉次数（>=0，只计管理员审核通过的投诉）；定级见 complaint_level_rank：有规模(>=50人)按每百人投诉率 0.5/1.5/3.0% 分档，否则按次数 0/1-2/3-5/6-9/>=10；职位列表按等级从优到劣返回';
