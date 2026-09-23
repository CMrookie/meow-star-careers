-- 线上视频面试：预约/状态/房间生命周期（音视频信令经现有 WebSocket 转发）
CREATE TABLE interviews (
    id             UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    application_id UUID NOT NULL REFERENCES applications(id) ON DELETE CASCADE,
    job_id         UUID NOT NULL REFERENCES jobs(id) ON DELETE CASCADE,
    company_id     UUID NOT NULL REFERENCES companies(id) ON DELETE CASCADE,
    interviewer_id UUID NOT NULL REFERENCES users(id) ON DELETE CASCADE,  -- 招聘者（发起方）
    interviewee_id UUID NOT NULL REFERENCES users(id) ON DELETE CASCADE,  -- 求职者
    status         TEXT NOT NULL DEFAULT 'invited'
                       CHECK (status IN ('invited','in_progress','finished','cancelled')),
    scheduled_at   TIMESTAMPTZ,   -- NULL = 即时面试；非空 = 预约时间（到点后方可进入）
    started_at     TIMESTAMPTZ,
    ended_at       TIMESTAMPTZ,
    created_by     UUID NOT NULL REFERENCES users(id),
    created_at     TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at     TIMESTAMPTZ NOT NULL DEFAULT now()
);

-- 每个投递同时只允许一个进行中/待开始的面试
CREATE UNIQUE INDEX idx_interviews_active_application
    ON interviews (application_id) WHERE status IN ('invited','in_progress');

CREATE INDEX idx_interviews_interviewee ON interviews (interviewee_id, status);
CREATE INDEX idx_interviews_interviewer ON interviews (interviewer_id, status);

CREATE TRIGGER trg_interviews_set_updated_at
    BEFORE UPDATE ON interviews
    FOR EACH ROW
    EXECUTE FUNCTION set_updated_at();

COMMENT ON TABLE interviews IS '线上面试（房间）：关联投递，双角色（招聘者发起/求职者参加），含预约与状态流转';
COMMENT ON COLUMN interviews.status IS 'invited=待开始/已邀请 | in_progress=进行中 | finished=已结束 | cancelled=已取消';
COMMENT ON COLUMN interviews.scheduled_at IS '预约开始时间；NULL 表示即时面试（创建后即可进入）';
COMMENT ON INDEX idx_interviews_active_application IS '同一投递同时只允许一个活跃面试（invited/in_progress）';
