-- 企业“提供职位的所在地”：用于职位详情展示/地图定位（街道级，可空）
ALTER TABLE companies ADD COLUMN address TEXT;
COMMENT ON COLUMN companies.address IS '详细地址（街道级，如 南山区科技园X路X号）；用于职位详情的地图定位';
