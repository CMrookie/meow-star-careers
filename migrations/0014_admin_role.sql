-- 平台管理员角色：投诉（举报）审核必须由 admin 执行。
--
-- 背景：handlers/complaints.rs 的审核/全量查询分支判断 `role == "admin"`，但此前
-- users.role 的 CHECK 只允许 seeker/recruiter，且注册接口只能创建这两种角色 ——
-- 也就是说「举报审核」链路在当时是**不可达**的（库里造不出 admin）。本迁移放开约束。
--
-- 安全约定：admin **不可自助注册**（POST /auth/register 显式拒绝 role=admin），
-- 只能由服务端启动时按环境变量 ADMIN_PHONE / ADMIN_PASSWORD 幂等引导创建
-- （见 src/repositories/auth.rs::ensure_admin，以及 main.rs 的启动引导）。

ALTER TABLE users DROP CONSTRAINT chk_users_role;

ALTER TABLE users ADD CONSTRAINT chk_users_role
    CHECK (role IN ('seeker', 'recruiter', 'admin'));

COMMENT ON CONSTRAINT chk_users_role ON users IS
    '用户角色：seeker 求职者 / recruiter 招聘者（绑定企业）/ admin 平台管理员（仅服务端引导创建，不可自助注册）';

COMMENT ON COLUMN users.role IS
    '角色：seeker 求职者 / recruiter 招聘者 / admin 平台管理员（负责投诉审核等平台级操作）';
