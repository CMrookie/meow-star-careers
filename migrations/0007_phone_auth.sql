-- 纯手机号账号：users 增加 phone（唯一、可空以兼容历史邮箱账号），email 转为可空
ALTER TABLE users ADD COLUMN phone TEXT;

-- 历史账号以 email 为主；手机号注册的用户 email 为空。email 唯一约束保留（NULL 不参与唯一比较）。
ALTER TABLE users ALTER COLUMN email DROP NOT NULL;

CREATE UNIQUE INDEX idx_users_phone_unique ON users (phone) WHERE phone IS NOT NULL;

-- ============ 数据库元数据注释（COMMENT ON）============
COMMENT ON COLUMN users.phone IS '注册手机号（11 位数字，唯一；邮箱账号注册的历史用户为空）';
COMMENT ON COLUMN users.email IS '联系邮箱（可空；新注册账号为手机号，邮箱仅作历史/扩展用途）';
COMMENT ON INDEX idx_users_phone_unique IS '手机号唯一索引（部分索引，忽略 NULL）';
