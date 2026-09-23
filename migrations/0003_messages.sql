-- 即时通信：一对一私聊会话 + 消息表

-- 会话：固定 user_lo < user_hi 保证 (a,b) 与 (b,a) 是同一会话
CREATE TABLE conversations (
    id         UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    user_lo    UUID NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    user_hi    UUID NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    CONSTRAINT chk_conversations_lo_lt_hi CHECK (user_lo < user_hi),
    CONSTRAINT uq_conversations_pair UNIQUE (user_lo, user_hi)
);

CREATE INDEX idx_conversations_user_lo ON conversations (user_lo, updated_at DESC);
CREATE INDEX idx_conversations_user_hi ON conversations (user_hi, updated_at DESC);

-- 消息：会话内自增 id（天然有序，可作游标），read_at 为收件人已读时间
CREATE TABLE messages (
    id              BIGINT GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
    conversation_id UUID NOT NULL REFERENCES conversations(id) ON DELETE CASCADE,
    sender_id       UUID NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    recipient_id    UUID NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    content         TEXT NOT NULL,
    created_at      TIMESTAMPTZ NOT NULL DEFAULT now(),
    read_at         TIMESTAMPTZ,
    CONSTRAINT chk_messages_no_self CHECK (sender_id <> recipient_id)
);

CREATE INDEX idx_messages_conversation_created ON messages (conversation_id, id);
CREATE INDEX idx_messages_unread ON messages (conversation_id, recipient_id)
    WHERE read_at IS NULL;

-- ============ 数据库元数据注释（COMMENT ON）============
COMMENT ON TABLE conversations IS '一对一私聊会话；以 (user_lo, user_hi) 唯一约束保证两人只有一条会话';
COMMENT ON COLUMN conversations.id IS '会话主键（UUID）';
COMMENT ON COLUMN conversations.user_lo IS '参与用户较小 id（规范化后的 a）';
COMMENT ON COLUMN conversations.user_hi IS '参与用户较大 id（规范化后的 b），user_lo < user_hi';
COMMENT ON COLUMN conversations.created_at IS '会话创建时间（UTC）';
COMMENT ON COLUMN conversations.updated_at IS '会话最近活跃时间：每条新消息会刷新（会话列表按此倒序）';
COMMENT ON CONSTRAINT chk_conversations_lo_lt_hi ON conversations IS '强制 user_lo < user_hi，保证 (a,b) 与 (b,a) 指向同一会话';
COMMENT ON CONSTRAINT uq_conversations_pair ON conversations IS '同一对用户只能有一个会话';

COMMENT ON TABLE messages IS '私聊消息（会话内自增 id 天然有序，可作为分页游标）';
COMMENT ON COLUMN messages.id IS '消息自增主键（会话内越大越新）';
COMMENT ON COLUMN messages.conversation_id IS '所属会话（级联删除）';
COMMENT ON COLUMN messages.sender_id IS '发送者用户 id';
COMMENT ON COLUMN messages.recipient_id IS '接收者用户 id（= 会话对端，便于未读计数）';
COMMENT ON COLUMN messages.content IS '文本内容（≤2000 字符）';
COMMENT ON COLUMN messages.created_at IS '发送时间（UTC）';
COMMENT ON COLUMN messages.read_at IS '接收者已读时间；NULL=未读（配合部分索引统计未读）';
COMMENT ON CONSTRAINT chk_messages_no_self ON messages IS '禁止给自己发消息';
