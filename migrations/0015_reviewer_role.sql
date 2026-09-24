-- 审核专用账号：把「平台管理」与「内容审核」拆成两种角色。
--
--   admin    = 平台管理：管理账号（创建/启用禁用/重置密码/删除审核账号）、用户管理；
--              作为超级角色也可代审；
--   reviewer = 审核专用账号：只能查看投诉列表与执行审核（举报审核），由 admin 创建管理。
--
-- 两者都**不可自助注册**（POST /auth/register 显式拒绝），admin 由服务端
-- ADMIN_PHONE / ADMIN_PASSWORD 引导（见 0014_admin_role.sql），reviewer 由 admin 创建
-- （POST /api/v1/reviewers）。
--
-- 「多个审核同时在线」不靠额外机制：令牌是**按账号、多条并存**的
-- （auth_tokens 每行一个令牌，登录不会踢掉其它令牌），因此多个审核账号可同时登录，
-- 同一账号也可多端登录；审核留痕在 complaints.reviewed_by（+ reviewed_by_name）。

ALTER TABLE users DROP CONSTRAINT chk_users_role;

ALTER TABLE users ADD CONSTRAINT chk_users_role
    CHECK (role IN ('seeker', 'recruiter', 'admin', 'reviewer'));

COMMENT ON CONSTRAINT chk_users_role ON users IS
    '用户角色：seeker 求职者 / recruiter 招聘者（绑定企业）/ admin 平台管理（管理员，仅服务端引导创建）/ reviewer 审核专用账号（投诉审核，仅 admin 可创建管理）';

COMMENT ON COLUMN users.role IS
    '角色：seeker 求职者 / recruiter 招聘者 / admin 平台管理（账号与用户管理，可代审）/ reviewer 审核专用账号（仅审核投诉）';
