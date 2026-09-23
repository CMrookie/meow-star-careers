-- 投诉业务流：仅实际沟通后的求职者可就用人单位发起投诉；管理员审核通过后才累计 complaint_count
CREATE TABLE complaints (
    id            UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    company_id    UUID NOT NULL REFERENCES companies(id) ON DELETE CASCADE,
    complainant_id UUID NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    evidence      TEXT NOT NULL,             -- 有效证据说明（要求内容>=20字符，可补充沟通事实）
    status        TEXT NOT NULL DEFAULT 'pending'
                      CHECK (status IN ('pending','approved','rejected')),
    review_note   TEXT,
    reviewed_by   UUID REFERENCES users(id),
    created_at    TIMESTAMPTZ NOT NULL DEFAULT now(),
    reviewed_at   TIMESTAMPTZ,
    updated_at    TIMESTAMPTZ NOT NULL DEFAULT now()
);
CREATE INDEX idx_complaints_company ON complaints (company_id, status);
CREATE INDEX idx_complaints_complainant ON complaints (complainant_id, status);
CREATE TRIGGER trg_complaints_set_updated_at
    BEFORE UPDATE ON complaints
    FOR EACH ROW EXECUTE FUNCTION set_updated_at();

COMMENT ON TABLE complaints IS '用人单位投诉（求职者发起->管理员审核->通过后生效累加 complaints_count）';
COMMENT ON COLUMN complaints.evidence IS '投诉证据说明（必填，>=20 字符；可附实际沟通事实）';
COMMENT ON COLUMN complaints.status IS 'pending=待审核 / approved=已通过并生效 / rejected=已驳回';
COMMENT ON COLUMN complaints.review_note IS '管理员审核备注（驳回原因等）';
