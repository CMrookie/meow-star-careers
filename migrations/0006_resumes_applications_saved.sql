-- 招聘求职：简历 / 投递 / 职位收藏
CREATE TABLE resumes (
    id         UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    user_id    UUID NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    full_name  TEXT NOT NULL,
    title      TEXT NOT NULL,
    phone      TEXT,
    email      TEXT,
    years      INTEGER,
    education  TEXT,
    skills     TEXT,
    summary    TEXT,
    is_public  BOOLEAN NOT NULL DEFAULT FALSE,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE INDEX idx_resumes_user ON resumes (user_id);
CREATE INDEX idx_resumes_public ON resumes (is_public);

CREATE TABLE applications (
    id           UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    job_id       UUID NOT NULL REFERENCES jobs(id) ON DELETE CASCADE,
    seeker_id    UUID NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    resume_id    UUID REFERENCES resumes(id) ON DELETE SET NULL,
    cover_letter TEXT,
    status       TEXT NOT NULL DEFAULT 'pending',
    created_at   TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at   TIMESTAMPTZ NOT NULL DEFAULT now(),
    CONSTRAINT chk_applications_status CHECK (
        status IN ('pending', 'viewed', 'interviewing', 'offered', 'rejected', 'withdrawn')
    ),
    CONSTRAINT uq_applications_job_seeker UNIQUE (job_id, seeker_id)
);

CREATE INDEX idx_applications_seeker ON applications (seeker_id, created_at DESC);
CREATE INDEX idx_applications_job ON applications (job_id, created_at DESC);

CREATE TABLE saved_jobs (
    user_id    UUID NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    job_id     UUID NOT NULL REFERENCES jobs(id) ON DELETE CASCADE,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    PRIMARY KEY (user_id, job_id)
);

CREATE INDEX idx_saved_jobs_job ON saved_jobs (job_id);

-- ============ 数据库元数据注释（COMMENT ON）============
COMMENT ON TABLE resumes IS '求职者简历；is_public=false 时仅本人可见，招聘者按不存在处理';
COMMENT ON COLUMN resumes.id IS '简历主键（UUID）';
COMMENT ON COLUMN resumes.user_id IS '所属求职者（级联删除）';
COMMENT ON COLUMN resumes.full_name IS '姓名';
COMMENT ON COLUMN resumes.title IS '期望职位';
COMMENT ON COLUMN resumes.phone IS '联系电话';
COMMENT ON COLUMN resumes.email IS '联系邮箱';
COMMENT ON COLUMN resumes.years IS '工作年限';
COMMENT ON COLUMN resumes.education IS '学历（自由文本）';
COMMENT ON COLUMN resumes.skills IS '技能关键词（检索用）';
COMMENT ON COLUMN resumes.summary IS '自我评价/简介';
COMMENT ON COLUMN resumes.is_public IS '是否公开给招聘者检索与查看';
COMMENT ON COLUMN resumes.created_at IS '创建时间（UTC）';
COMMENT ON COLUMN resumes.updated_at IS '最近更新时间（UTC）';

COMMENT ON TABLE applications IS '投递记录：一名求职者对一个职位只能投递一次';
COMMENT ON COLUMN applications.id IS '投递主键（UUID）';
COMMENT ON COLUMN applications.job_id IS '目标职位（级联删除）';
COMMENT ON COLUMN applications.seeker_id IS '投递的求职者（级联删除）';
COMMENT ON COLUMN applications.resume_id IS '本次投递所用简历（删除后置 NULL）';
COMMENT ON COLUMN applications.cover_letter IS '求职信（可选）';
COMMENT ON COLUMN applications.status IS '状态：pending 待处理 / viewed 已查看 / interviewing 面试中 / offered 已录用 / rejected 已拒绝 / withdrawn 已撤回';
COMMENT ON COLUMN applications.created_at IS '投递时间（UTC）';
COMMENT ON COLUMN applications.updated_at IS '状态最近更新时间（UTC）';
COMMENT ON CONSTRAINT chk_applications_status ON applications IS '状态只允许六种取值';
COMMENT ON CONSTRAINT uq_applications_job_seeker ON applications IS '同一职位同一求职者只允许一条投递';

COMMENT ON TABLE saved_jobs IS '职位收藏（求职者收藏心仪职位）';
COMMENT ON COLUMN saved_jobs.user_id IS '收藏用户（级联删除）';
COMMENT ON COLUMN saved_jobs.job_id IS '被收藏职位（级联删除）';
COMMENT ON COLUMN saved_jobs.created_at IS '收藏时间（UTC）';
