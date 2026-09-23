-- 预留：移动端系统推送注册（FCM/APNs 接入后使用）
CREATE TABLE device_push_tokens (
    id         UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    user_id    UUID NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    platform   TEXT NOT NULL CHECK (platform IN ('android','ios','web')),
    token      TEXT NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now()
);
CREATE UNIQUE INDEX idx_device_push_tokens_user_platform
    ON device_push_tokens (user_id, platform, token);
COMMENT ON TABLE device_push_tokens IS '系统推送注册令牌（预留；接入 FCM/APNs 后由推送服务读取投递）';
COMMENT ON COLUMN device_push_tokens.platform IS 'android / ios / web';
