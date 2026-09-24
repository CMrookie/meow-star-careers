-- 多审核并行：投诉「认领即锁定」（claim / lock）
--
-- 背景：审核账号可以有多个，且令牌按账号并存、登录互不踢，因此多个审核会同时在线。
-- 若两人同时打开同一条投诉，既重复劳动，又可能互相覆盖结论。于是引入认领锁：
--
--   locked_by         当前持锁的审核账号（NULL = 空闲）
--   locked_at         本次获得锁的时间
--   lock_expires_at   锁到期时间；到期即视为空闲，任何人都可重新认领
--                     （审核端在弹窗打开期间定期续约来维持锁）
--   review_started_at 首次认领时间，用于统计「接单 -> 审结」的处理时长；
--                     与锁不同，它一旦写入就不再清除，是审核及时性的度量起点
--
-- 一致性：
--   - 三个锁字段要么全空、要么全非空（chk_complaints_lock_all_or_none），杜绝「半锁」；
--   - 审结（approved/rejected）时清空三个锁字段，但保留 review_started_at；
--   - 锁只在 status='pending' 时才有意义：认领/续约的 UPDATE 都带 status 条件。
--
-- 说明：本迁移同时引入只读视图 complaint_views，把「投诉 + 企业名 + 投诉人名 +
-- 审核人名 + 持锁人名 + 锁是否有效」的联表与计算收敛到一处，避免仓储层多处复制 SQL。

ALTER TABLE complaints
    ADD COLUMN locked_by UUID REFERENCES users(id) ON DELETE SET NULL,
    ADD COLUMN locked_at TIMESTAMPTZ,
    ADD COLUMN lock_expires_at TIMESTAMPTZ,
    ADD COLUMN review_started_at TIMESTAMPTZ;

ALTER TABLE complaints
    ADD CONSTRAINT chk_complaints_lock_all_or_none
    CHECK (
        (locked_by IS NULL AND locked_at IS NULL AND lock_expires_at IS NULL)
        OR (locked_by IS NOT NULL AND locked_at IS NOT NULL AND lock_expires_at IS NOT NULL)
    );

-- 待审队列里「哪些空闲」与「某人手里有几条」都走部分索引（只索引持锁行）
CREATE INDEX idx_complaints_lock_expires
    ON complaints (lock_expires_at)
    WHERE locked_by IS NOT NULL;

CREATE INDEX idx_complaints_locked_by
    ON complaints (locked_by)
    WHERE locked_by IS NOT NULL;

COMMENT ON COLUMN complaints.locked_by IS
    '当前认领该投诉的审核账号（NULL=空闲）；审结时清空';
COMMENT ON COLUMN complaints.locked_at IS
    '本次获得锁的时间；续约不改写该值';
COMMENT ON COLUMN complaints.lock_expires_at IS
    '锁到期时间；到期后视为空闲可被他人重新认领（审核端在弹窗打开期间定期续约）';
COMMENT ON COLUMN complaints.review_started_at IS
    '首次认领时间（接单时刻），用于统计「接单 -> 审结」处理时长；审结后仍保留，锁字段则清空';
COMMENT ON CONSTRAINT chk_complaints_lock_all_or_none ON complaints IS
    '锁字段全空或全非空：避免出现只有部分锁字段的中间状态';

-- 只读视图：投诉视图 + 锁状态。lock_active 由数据库按当前时间计算，前端直接展示。
CREATE VIEW complaint_views AS
SELECT cp.id,
       cp.company_id,
       c.name                        AS company_name,
       cp.complainant_id,
       u.name                        AS complainant_name,
       cp.evidence,
       cp.status,
       cp.review_note,
       cp.reviewed_by,
       reviewer.name                 AS reviewed_by_name,
       cp.locked_by,
       locker.name                   AS locked_by_name,
       cp.locked_at,
       cp.lock_expires_at,
       (cp.locked_by IS NOT NULL AND cp.lock_expires_at > now()) AS lock_active,
       cp.review_started_at,
       cp.created_at,
       cp.reviewed_at,
       cp.updated_at
  FROM complaints cp
  JOIN companies c ON c.id = cp.company_id
  JOIN users u ON u.id = cp.complainant_id
  LEFT JOIN users reviewer ON reviewer.id = cp.reviewed_by
  LEFT JOIN users locker ON locker.id = cp.locked_by;

COMMENT ON VIEW complaint_views IS
    '投诉视图：联表企业名/投诉人名/审核人名/持锁人名，并计算锁是否仍有效（lock_active）';
