-- 补齐历史数据缺失的企业规模（staff_size）。
--
-- 背景：staff_size 在 0017 才引入，此前创建的企业一律为 NULL —— 定级会自动退回
-- 「投诉次数」口径，于是看不出「按公司规模折算」的效果（联调库里 5/5 家企业都缺）。
--
-- 本迁移只填**联调/演示数据**里这几家同名企业（幂等：仅当 staff_size IS NULL）。
-- 这些人数是演示用的申报值，不是核验值；生产数据应以企业实际申报 + 平台核验为准
-- （核验口径见 README「定级依据」第 5 条）。
--
-- 若你的联调库用了别家名称，用一句 SQL 即可补齐（或走「招聘者注册时的 company.staffSize」）：
--   UPDATE companies SET staff_size = 1234 WHERE name = '你的公司名';

UPDATE companies SET staff_size = 2000 WHERE staff_size IS NULL AND name = '星河科技';
UPDATE companies SET staff_size = 120  WHERE staff_size IS NULL AND name = '云图数据';
UPDATE companies SET staff_size = 300  WHERE staff_size IS NULL AND name = '蓝海医疗';
UPDATE companies SET staff_size = 80   WHERE staff_size IS NULL AND name = '联调科技';
UPDATE companies SET staff_size = 500  WHERE staff_size IS NULL AND name = '地图测试科技';

COMMENT ON COLUMN companies.staff_size IS
    '企业规模（员工人数，企业申报；NULL = 未申报）。投诉定级 v2 的分母：>=50 人时按每百人投诉率定级，否则退回投诉次数口径（联调演示数据由 0019_backfill_staff_size.sql 补）';
