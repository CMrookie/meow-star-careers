-- 认证：users 增加密码哈希列 + 登录令牌表（存哈希，可撤销、可过期）
ALTER TABLE users ADD COLUMN password_hash TEXT;

CREATE TABLE auth_tokens (
    token_hash TEXT PRIMARY KEY,           -- sha256(bearer_token) 十六进制
    user_id    UUID NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    expires_at TIMESTAMPTZ NOT NULL,
    revoked_at TIMESTAMPTZ                 -- 非空表示已登出/撤销
);

CREATE INDEX idx_auth_tokens_user_id ON auth_tokens (user_id);
CREATE INDEX idx_auth_tokens_expires  ON auth_tokens (expires_at);

-- ============ 数据库元数据注释（COMMENT ON）============
COMMENT ON COLUMN users.password_hash IS '密码哈希（argon2id PHC 字符串）；仅通过 auth/register 注册的用户才有，旧直建账号为 NULL 无法登录';
COMMENT ON TABLE auth_tokens IS '登录会话令牌表（只存 sha256 摘要，不存明文）';
COMMENT ON COLUMN auth_tokens.token_hash IS '令牌摘要（sha256(bearer_token) 的十六进制，主键）';
COMMENT ON COLUMN auth_tokens.user_id IS '所属用户（级联删除）';
COMMENT ON COLUMN auth_tokens.created_at IS '签发时间（UTC）';
COMMENT ON COLUMN auth_tokens.expires_at IS '过期时间（默认 30 天）';
COMMENT ON COLUMN auth_tokens.revoked_at IS '撤销时间（登出置位；NULL=仍有效）';
