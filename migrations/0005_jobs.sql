-- 招聘求职：职位表
CREATE TABLE jobs (
    id           UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    company_id   UUID NOT NULL REFERENCES companies(id) ON DELETE CASCADE,
    title        TEXT NOT NULL,
    description  TEXT NOT NULL,
    requirements TEXT,
    location     TEXT,
    salary_min   INTEGER,
    salary_max   INTEGER,
    job_type     TEXT NOT NULL DEFAULT 'full_time',
    experience   TEXT,
    education    TEXT,
    is_active    BOOLEAN NOT NULL DEFAULT TRUE,
    created_by   UUID REFERENCES users(id) ON DELETE SET NULL,
    created_at   TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at   TIMESTAMPTZ NOT NULL DEFAULT now(),
    CONSTRAINT chk_jobs_salary CHECK (salary_min IS NULL OR salary_max IS NULL OR salary_min <= salary_max),
    CONSTRAINT chk_jobs_job_type CHECK (job_type IN ('full_time', 'part_time', 'contract', 'intern'))
);

CREATE INDEX idx_jobs_company ON jobs (company_id);
CREATE INDEX idx_jobs_active_created ON jobs (is_active, created_at DESC);
CREATE INDEX idx_jobs_title ON jobs (title);
CREATE INDEX idx_jobs_location ON jobs (location);

-- ============ 数据库元数据注释（COMMENT ON）============
COMMENT ON TABLE jobs IS '职位表：由招聘者（本企业）发布，is_active=false 视为下架';
COMMENT ON COLUMN jobs.id IS '职位主键（UUID）';
COMMENT ON COLUMN jobs.company_id IS '发布企业（级联删除）';
COMMENT ON COLUMN jobs.title IS '职位名称，如「后端工程师」';
COMMENT ON COLUMN jobs.description IS '职位描述（JD）';
COMMENT ON COLUMN jobs.requirements IS '任职要求';
COMMENT ON COLUMN jobs.location IS '工作地点（城市）';
COMMENT ON COLUMN jobs.salary_min IS '月薪下限（元，可空）';
COMMENT ON COLUMN jobs.salary_max IS '月薪上限（元，可空）';
COMMENT ON COLUMN jobs.job_type IS '工作性质：full_time / part_time / contract / intern';
COMMENT ON COLUMN jobs.experience IS '经验要求（自由文本，如 3-5年）';
COMMENT ON COLUMN jobs.education IS '学历要求（自由文本，如 本科）';
COMMENT ON COLUMN jobs.is_active IS '是否在招（上架）；公开搜索仅返回 true 的职位';
COMMENT ON COLUMN jobs.created_by IS '发布者用户 id（级联 SET NULL）';
COMMENT ON COLUMN jobs.created_at IS '发布时间（UTC）';
COMMENT ON COLUMN jobs.updated_at IS '最近更新时间（UTC）';
COMMENT ON CONSTRAINT chk_jobs_salary ON jobs IS '薪资区间下限不得超过上限';
COMMENT ON CONSTRAINT chk_jobs_job_type ON jobs IS '工作性质取值为受支持的四类';
