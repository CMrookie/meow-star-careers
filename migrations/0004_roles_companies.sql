-- 招聘求职：用户角色 + 企业表
-- role: seeker=求职者 / recruiter=招聘者（隶属于一个 company）

CREATE TABLE companies (
    id          UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    name        TEXT NOT NULL,
    industry    TEXT,
    description TEXT,
    location    TEXT,
    website     TEXT,
    logo_url    TEXT,
    is_active   BOOLEAN NOT NULL DEFAULT TRUE,
    created_by  UUID REFERENCES users(id) ON DELETE SET NULL,
    created_at  TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at  TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE INDEX idx_companies_name ON companies (name);

ALTER TABLE users ADD COLUMN role TEXT NOT NULL DEFAULT 'seeker';
ALTER TABLE users ADD CONSTRAINT chk_users_role CHECK (role IN ('seeker', 'recruiter'));

ALTER TABLE users ADD COLUMN company_id UUID REFERENCES companies(id) ON DELETE SET NULL;
CREATE INDEX idx_users_company ON users (company_id);

-- ============ 数据库元数据注释（COMMENT ON）============
COMMENT ON TABLE companies IS '企业（招聘单位）表；招聘者账号经 company_id 归属企业';
COMMENT ON COLUMN companies.id IS '企业主键（UUID）';
COMMENT ON COLUMN companies.name IS '企业名称';
COMMENT ON COLUMN companies.industry IS '所属行业（自由文本）';
COMMENT ON COLUMN companies.description IS '企业介绍';
COMMENT ON COLUMN companies.location IS '所在城市/地区';
COMMENT ON COLUMN companies.website IS '官网地址';
COMMENT ON COLUMN companies.logo_url IS 'Logo 图片地址';
COMMENT ON COLUMN companies.is_active IS '是否启用（默认启用）';
COMMENT ON COLUMN companies.created_by IS '创建该企业的招聘者用户 id（级联 SET NULL）';
COMMENT ON COLUMN companies.created_at IS '创建时间（UTC）';
COMMENT ON COLUMN companies.updated_at IS '最近更新时间（UTC）';

COMMENT ON COLUMN users.role IS '账号角色：seeker=求职者 / recruiter=招聘者';
COMMENT ON COLUMN users.company_id IS '招聘者所属企业 id（求职者为 NULL）';
COMMENT ON CONSTRAINT chk_users_role ON users IS '角色只允许 seeker / recruiter';
