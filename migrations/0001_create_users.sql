-- 用户表（示例业务模型）
CREATE TABLE users (
    id         UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    email      TEXT NOT NULL UNIQUE,
    name       TEXT NOT NULL,
    is_active  BOOLEAN NOT NULL DEFAULT TRUE,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

-- 更新时自动维护 updated_at
CREATE OR REPLACE FUNCTION set_updated_at()
RETURNS TRIGGER AS $$
BEGIN
    NEW.updated_at = now();
    RETURN NEW;
END;
$$ LANGUAGE plpgsql;

CREATE TRIGGER trg_users_set_updated_at
    BEFORE UPDATE ON users
    FOR EACH ROW
    EXECUTE FUNCTION set_updated_at();

-- ============ 数据库元数据注释（COMMENT ON）============
COMMENT ON TABLE users IS '用户账号表（招聘求职平台通用账号；求职者/招聘者见 role 字段，0004）';
COMMENT ON COLUMN users.id IS '用户主键（UUID）';
COMMENT ON COLUMN users.email IS '登录邮箱，唯一（应用层统一小写存储）';
COMMENT ON COLUMN users.name IS '显示昵称/姓名';
COMMENT ON COLUMN users.is_active IS '是否启用（禁用后不可登录）';
COMMENT ON COLUMN users.created_at IS '创建时间（UTC）';
COMMENT ON COLUMN users.updated_at IS '最近更新时间（UTC），由触发器自动维护';
COMMENT ON FUNCTION set_updated_at() IS '通用触发器函数：UPDATE 时自动把 updated_at 置为 now()';
