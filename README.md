# meow-star-careers · 招聘求职系统服务端

**meow-star-careers** 是模块化的 **Actix Web + PostgreSQL** 招聘求职平台后端：注册登录（求职者/招聘者/审核账号）、职位发布与搜索、简历管理、在线投递与状态流转、职位收藏、投诉（举报）与审核，另含一对一实时私聊。

- 框架：actix-web 4（Rust 2024 edition），WebSocket 使用官方 actix-ws（无 actor 流式方案）
- 数据库：PostgreSQL，通过 sqlx（连接池 + 内嵌迁移）
- **API 文档自动生成**：utoipa 注解 -> OpenAPI 3，随路由在 `/swagger-ui/` 提供交互式 UI（含 bearerAuth 安全方案）
- **模块化分层**：`models` / `repositories` / `handlers` / `db` / `ws` / `auth` / `security`，`main.rs` 保持精简
- **统一错误处理**：单一 `AppError` + `ResponseError`，所有错误输出一致 JSON（`code` + `message`）并集中记录日志；5xx 对客户端脱敏
- **日志记录**：tracing + tracing-subscriber（`RUST_LOG` 过滤），`TracingLogger` 输出结构化请求日志（含 request id）
- **角色模型**：单 `users` 表 + `role`（seeker/recruiter/admin/reviewer），招聘者关联 `companies`；注册时招聘者同事务创建企业；**admin 与 reviewer 都不可自助注册** —— admin 由服务端按 `ADMIN_PHONE` 幂等引导，reviewer（审核专用账号）由 admin 通过 `/reviewers` 创建管理
- **搜索**：PostgreSQL ILIKE/范围过滤 + 分页（零额外服务）
- **测试**：`src/tests/` 集成测试（`#[sqlx::test]` 每个用例独立建库），覆盖鉴权 / 举报审核 / 审核账号 / 列表排序

## 目录结构

```
├── Cargo.toml
├── .env.example               # 环境变量样例（默认连本机 pgdb）
├── migrations/
│   ├── 0001_create_users.sql     # 用户表
│   ├── 0002_auth.sql             # password_hash + auth_tokens
│   ├── 0003_messages.sql         # 私聊 conversations + messages
│   ├── 0004_roles_companies.sql  # 角色 + 企业表
│   ├── 0005_jobs.sql             # 职位表
│   ├── 0006_resumes_applications_saved.sql  # 简历/投递/收藏
│   ├── ...                       # 0007-0012：手机号登录/面试/推送/企业地址/投诉
│   ├── 0013_complaint_level_order.sql  # 投诉等级函数（列表按等级排序）
│   ├── 0014_admin_role.sql       # 平台管理角色（admin 不可自助注册）
│   ├── 0015_reviewer_role.sql    # 审核专用账号（reviewer，由 admin 创建管理）
│   ├── 0016_complaint_review_lock.sql  # 投诉认领锁 + complaint_views 只读视图
│   └── 0017_company_staff_size.sql     # 企业规模 + 定级 v2（每百人投诉率）
└── src/
    ├── main.rs                   # 入口（配置/迁移/管理员引导/启动）
    ├── config.rs / logging.rs / db.rs / state.rs
    ├── security.rs               # argon2id / 令牌 / 手机号校验
    ├── auth.rs                   # AuthenticatedUser 提取器
    ├── error.rs                  # 统一错误
    ├── ws.rs                     # WebSocket 实时中枢（私聊）
    ├── openapi.rs / app.rs
    ├── models/     # user/auth/chat/company/job/resume/application/complaint/pagination/stats
    ├── repositories/  # user/auth/chat/company/job/resume/application/complaint/stats
    ├── handlers/     # health/auth/users/reviewers/companies/jobs/resumes/applications/chat/complaints/stats
    └── tests/        # 集成测试：鉴权 / 举报审核 / 审核账号 / 并行锁定 / 平台统计 / 定级 v2 / 列表排序
```

## 快速开始

依赖：Rust ≥ 1.85、正在运行的 PostgreSQL 容器 **pgdb**（宿主 5432，超级用户 postgres/password）。

```bash
# PostgreSQL：直接使用现有 pgdb；首次先建项目库（可重复执行）
docker exec pgdb psql -U postgres -h 127.0.0.1 -c "CREATE DATABASE appdb OWNER postgres;" 2>/dev/null || true

cp .env.example .env            # 默认 DATABASE_URL 已指向 pgdb 的 appdb
cargo run                       # 启动时自动执行迁移（建表 + COMMENT 注释）
```

访问：<http://127.0.0.1:8080/swagger-ui/>

## 角色与账号

> 账号体系：**手机号 + 密码**（中国大陆 11 位手机号，无需短信验证码）。
> 兼容说明：早期由邮箱注册的历史账号仍保留 `email`，登录一律按手机号匹配；`users.phone` 唯一。

- 求职者：`role=seeker`（默认）——可浏览/搜索职位、收藏、维护简历、投递与撤回；
- 招聘者：`role=recruiter` + `company` ——发布/上下架/编辑职位，查看并推进投递状态，检索公开简历；
- 平台管理：`role=admin` ——**管理角色**：管理账号（创建/启停/改密/删除审核账号）、用户管理；作为超级角色也可代审投诉。**只能由服务端引导创建**：
  `POST /auth/register` 显式拒绝 `role=admin`（防公开接口提权），需要在 `.env` 里配置
  `ADMIN_PHONE` + `ADMIN_PASSWORD`（可选 `ADMIN_NAME`），服务启动时幂等创建：
  - 手机号不存在 → 新建 `admin` 账号；
  - 已是 `admin` → 保持原密码不动（不会每次启动静默改密）；
  - 已被其它角色占用 → 启动失败（**绝不把既有账号提权**）。
- 审核专用账号：`role=reviewer` ——**只做举报审核**：查看投诉队列、通过/驳回投诉；
  不能发职位、投递、维护简历，也不能管理账号（`/reviewers` 仅 admin）。同样**不可自助注册**，
  由 admin 通过 `POST /api/v1/reviewers` 创建（手机号 + 初始密码），并可随时启用/禁用/改密/删除。
  多个审核账号可**同时登录**、并行审核；同一账号也支持多端同时在线，审核记录通过
  `reviewedBy` / `reviewedByName` 留痕，便于追溯是谁审的。
- 注册接口：`POST /api/v1/auth/register`，请求体：

```jsonc
// 求职者
{ "phone": "13800138000", "name": "Alice", "password": "secret123" }
// 招聘者
{ "phone": "13900000001", "name": "HR", "password": "secret123",
  "role": "recruiter",
  // staffSize = 员工人数（企业申报，可选）：投诉定级 v2 的分母，≥50 人时按每百人投诉率定级
  "company": { "name": "公司名", "industry": "互联网", "location": "北京", "staffSize": 2000 } }
```

## REST 接口一览（除注明外均需 `Authorization: Bearer <token>`）

### 认证
| 方法 | 路径 | 说明 |
| --- | --- | --- |
| POST | `/api/v1/auth/register` | 注册（seeker / recruiter+company）→ `{token, user}` |
| POST | `/api/v1/auth/login` / `/logout` / GET `/auth/me` | 登录 / 登出 / 当前用户 |

### 用户与私聊
| 方法 | 路径 | 说明 |
| --- | --- | --- |
| GET/POST | `/api/v1/users` | 用户列表 / 创建（**仅平台管理员**） |
| GET/PUT | `/api/v1/users/{id}` | 查询 / 修改（**本人或平台管理员**；查他人一律 404） |
| DELETE | `/api/v1/users/{id}` | 删除用户（**仅平台管理员**，级联清理其数据） |
| POST/GET | `/api/v1/conversations(/{id})`、`/messages`、`/read` | 一对一私聊（REST） |
| GET | `/ws?token=` | WebSocket 实时通道（消息自动入库并推送双方） |

### 企业
| 方法 | 路径 | 说明 |
| --- | --- | --- |
| GET | `/api/v1/companies/mine` | 我的企业（招聘者） |
| GET | `/api/v1/companies/{id}` | 企业公开信息（含 `complaintsCount` 与 `staffSize`：规模是定级 v2 的分母） |

### 投诉（举报）与审核
| 方法 | 路径 | 说明 |
| --- | --- | --- |
| POST | `/api/v1/companies/{id}/complaints` | 发起投诉（求职者；**须与该企业有过实际沟通**，证据 20-5000 字） |
| GET | `/api/v1/complaints/mine` | 我发起的投诉 |
| GET | `/api/v1/complaints` | 审核账号/管理员=全部（`?status=pending/approved/rejected`）、招聘者=本企业、求职者=本人 |
| POST | `/api/v1/complaints/{id}/claim` | **认领即锁定**：多审核并行时同一条只允许一个持锁人；重复调用=续约（租约 10 分钟） |
| POST | `/api/v1/complaints/{id}/release` | 释放认领锁：本人释放；`?force=true` 仅管理员（强制解锁他人）；已过期的锁任何人可清理 |
| POST | `/api/v1/complaints/{id}/review` | **审核账号或管理员**审核 `{approved, note?}`：通过才累计企业投诉次数（影响职位列表排序），驳回不累计；重复审核 409；**被他人持锁时 409**；留痕 `reviewedBy`/`reviewedByName` |

### 审核账号管理（仅平台管理员）
| 方法 | 路径 | 说明 |
| --- | --- | --- |
| GET | `/api/v1/reviewers` | 审核专用账号列表 |
| POST | `/api/v1/reviewers` | 创建 `{phone, name, password}` → `role=reviewer`，可直接登录审核 |
| POST | `/api/v1/reviewers/{id}/active` | 启用/禁用 `{isActive}`；禁用**立即踢下线**（既有会话全部失效） |
| POST | `/api/v1/reviewers/{id}/password` | 重置密码；旧会话与旧密码同时失效 |
| DELETE | `/api/v1/reviewers/{id}` | 删除审核账号（令牌等数据级联清理） |

### 平台统计
| 方法 | 路径 | 说明 |
| --- | --- | --- |
| GET | `/api/v1/stats/review` | 待审队列概览（待审/处理中/锁过期/超时/最久等待）+ 审核账号工作量。**管理员=全部审核账号，审核账号=只有自己** |
| GET | `/api/v1/stats/companies` | 用人单位优劣（`?limit=` 默认 200）：投诉结构、职位规模、等级序、每在招职位投诉率、综合质量分；**最差在前** |
| GET | `/api/v1/stats/seekers` | 求职用户分析：账号规模与活跃、近 12 个月注册趋势、简历/投递参与度、投递状态与分档分布 |

> **审核只记录及时性**：`/stats/review` 返回的是「提交→审结」总时长、「认领→审结」处理时长、
> 及时率（及时线 24h）与队列积压；**刻意不返回通过/驳回数量与结论分布**，避免用审核结论去评价审核人员。
> `/stats/companies` 与 `/stats/seekers` 只返回**聚合值**，不返回任何用户明细。

> 该组接口只认 `role=reviewer` 的账号，对其它角色一律 404（既防误伤普通用户，也防探测）。

### 职位
| 方法 | 路径 | 说明 |
| --- | --- | --- |
| GET | `/api/v1/jobs` | 搜索：`keyword` `location` `jobType` `salaryMin` `salaryMax` `page` `pageSize` |
| GET | `/api/v1/jobs/{id}` | 职位详情（在招；本企业招聘者可看已下架） |
| POST | `/api/v1/jobs` | 发布职位（招聘者） |
| GET | `/api/v1/jobs/my` | 我的（本企业）职位 |
| PUT/DELETE | `/api/v1/jobs/{id}` | 编辑 / 删除职位 |
| POST | `/api/v1/jobs/{id}/active` | 上架/下架 `{isActive}` |
| POST | `/api/v1/jobs/{id}/save` `/unsave` | 收藏 / 取消收藏 |
| GET | `/api/v1/saved-jobs` | 我的收藏 |

#### 职位列表排序契约：一律按用人单位投诉等级「从优到劣」（定级 v2：按公司规模折算的投诉率）

`GET /jobs`、`GET /jobs/my`、`GET /saved-jobs` 三个列表**都在服务端排序**，返回顺序即最终展示顺序：

```
ORDER BY complaint_level_rank(c.complaints_count, c.staff_size) ASC,   -- 等级 v2：有规模按每百人投诉率
         (complaint_rate_percent(..., c.staff_size) IS NULL) ASC,      -- 同等级内「有规模折算」的优先
         COALESCE(complaint_rate_percent(...), c.complaints_count) ASC,-- 再按率（无规模按次数）小者优先
         <时间> DESC,                                                   -- 搜索/本企业按发布；收藏按收藏时间
         j.id DESC                                                      -- 末位兜底，保证分页稳定
```

**为什么要按规模折算**：固定次数对规模大的公司不友好 —— 2000 人公司 20 起投诉只有 1.0%，
而 60 人公司 3 起投诉就是 5%，两者性质完全不同。

| 等级 | 每百人投诉率（规模 ≥50 人） | 次数兜底（未申报规模 / <50 人） | 颜色（App 展示） |
| --- | --- | --- | --- |
| 优秀 excellent | 0（分子为 0，两种口径都判优秀） | 0 次 | 绿 |
| 轻微 minor | ≤ **0.5%** | 1 – 2 次 | 蓝 |
| 预警 alert | ≤ **1.5%** | 3 – 5 次 | 橙黄 |
| 警告 warning | ≤ **3.0%** | 6 – 9 次 | 橙红 |
| 严重 severe | **> 3.0%** | 10 次及以上 | 红 |

- 分子 `companies.complaints_count` 只计**管理员审核通过**的投诉（见 `0012_complaints.sql`）；
  分母 `companies.staff_size` 是**企业申报**的员工人数（招聘者注册时的 `company.staffSize`，可为空）。
- 小样本退回次数：规模 < 50 人时「1 起投诉」就能把率抬到 2%+，波动太大容易冤枉小公司，
  因此这一档改用次数定级，并在 App 卡片上标明口径（`basisNote`）。
- 阈值与口径与求职 App **同一份**：`lib/ui/complaint_level.dart` 的 `assessComplaints` /
  `complaintRateThresholds = [0.5, 1.5, 3.0]` / `minStaffSizeForRate = 50`；服务端对应
  `migrations/0017_company_staff_size.sql` 的 `complaint_rate_percent(complaints, staff_size)`
  与 `complaint_level_rank(complaints, staff_size)`（均 `IMMUTABLE`，可在 `ORDER BY`/过滤/分组复用）。
  **改一侧必须同步改另一侧**，`tests/complaint_rate.rs` 用边界表把两边钉在一起。
- 相关响应字段：职位视图带 `companyStaffSize`（未申报为 `null`），企业视图带 `staffSize`
  —— 与 App 的 `JobView.companyStaffSize` / `Company.staffSize` 一一对应。
- 因为排序在服务端完成，**翻页是全局有序的**：第 2 页不会出现比第 1 页等级更优的职位，
  客户端不需要也不能再自行排序（客户端排序只对「页内」有效）。
- `GET /jobs/my`（本企业职位）企业规模与投诉次数必然相同，实际退化为按发布时间倒序。
- **排序代价（实测）**：等级由「企业」的投诉次数与规模决定、需要联表，因此无法靠单表索引直接有序。
  在 3 万条职位的本地压测里，执行计划为 `Hash Join + top-N heapsort`，耗时约 **47ms**
  （`LIMIT 20` 只保留前 N 条，内存不随结果集增长）；另外验证过在 `companies(complaints_count)`
  上建索引对该计划**没有任何改善**（同样计划、同样耗时），故未添加该索引。
  数据量进一步增大时的可选方向：把等级物化到职位行上、或对「优秀档」单独走一条快路径。

### 投递（求职者）
| 方法 | 路径 | 说明 |
| --- | --- | --- |
| POST | `/api/v1/jobs/{id}/apply` | 投递（`resumeId?` `coverLetter?`），一人一岗一次 |
| GET | `/api/v1/applications` | 我的投递（可按 `jobId` `status` 过滤） |
| POST | `/api/v1/applications/{id}/status` | 撤回 `{"status":"withdrawn"}` |

### 投递（招聘者收件箱）
| 方法 | 路径 | 说明 |
| --- | --- | --- |
| GET | `/api/v1/applications` | 本企业收到的投递 |
| GET | `/api/v1/applications/{id}` | 投递详情（含求职者联系方式） |
| POST | `/api/v1/applications/{id}/status` | 推进：`viewed` → `interviewing` → `offered` / `rejected` |

### 简历
| 方法 | 路径 | 说明 |
| --- | --- | --- |
| POST/GET | `/api/v1/resumes` | 新建 / 我的简历（求职者） |
| GET/PUT/DELETE | `/api/v1/resumes/{id}` | 详情 / 更新 / 删除（仅本人） |
| GET | `/api/v1/resumes/search?keyword=` | 检索**公开**简历（仅招聘者） |

> 隐私规则：非本人、非招聘者时，私有简历一律按 404 处理（不泄露存在性）。

### 线上面试
| 方法 | 路径 | 说明 |
| --- | --- | --- |
| POST | `/api/v1/interviews` | 发起（`{applicationId, scheduledAt?}`；scheduledAt 为空=即时，否则预约） |
| GET | `/api/v1/interviews` | 我参与的面试（双方视角一致） |
| GET | `/api/v1/interviews/{id}` | 面试详情 |
| POST | `/api/v1/interviews/{id}/start` | 进入房间前调用（到预约时间后 invited→in_progress） |
| POST | `/api/v1/interviews/{id}/finish` | 结束（in_progress→finished） |
| POST | `/api/v1/interviews/{id}/cancel` | 取消（invited→cancelled） |

### 系统
| 方法 | 路径 | 说明 |
| --- | --- | --- |
| GET | `/`、`/healthz` | 服务信息、健康检查（含 DB 探活） |
| GET | `/api-docs/openapi.json`、`/swagger-ui/` | OpenAPI 规范与交互式文档 |

## WebSocket 私聊协议

`ws://127.0.0.1:8080/ws?token=<登录token>`

```json
{"type": "send", "conversationId": "<会话id>", "body": "约个面试时间？"}  // 上行
{"type": "message", "conversationId": "...", "message": { ... }}         // 下行（推给双方）
{"type": "ack", "conversationId": "..."}
{"type": "signal", "interviewId": "<面试id>", "payload": {"kind":"joined"}}        // 上行：面试信令
{"type": "interview-signal", "interviewId": "...", "from": "<userId>", "payload": {...}} // 下行：转发给对端
{"type": "interview-updated", "interview": { ... }}                              // 下行：面试状态变化推送给双方
{"type": "error", "code": "...", "message": "..."}
```

## 快速体验（curl）

```bash
B=http://127.0.0.1:8080/api/v1

# 1) 注册求职者与招聘者（各自拿到 token）
SEEKER=$(curl -s -X POST $B/auth/register -H 'content-type: application/json' \
  -d '{"phone":"13800138000","name":"Alice","password":"secret123"}')
ST=$(echo "$SEEKER" | python3 -c 'import sys,json;print(json.load(sys.stdin)["token"])')

HR=$(curl -s -X POST $B/auth/register -H 'content-type: application/json' \
  -d '{"phone":"13900000001","name":"HR","password":"secret123","role":"recruiter",\
       "company":{"name":"星河科技","industry":"互联网","location":"北京"}}')
HT=$(echo "$HR" | python3 -c 'import sys,json;print(json.load(sys.stdin)["token"])')

# 2) 招聘者发布职位，求职者搜索 -> 投递
JOB=$(curl -s -X POST $B/jobs -H "Authorization: Bearer $HT" -H 'content-type: application/json' \
  -d '{"title":"后端工程师","description":"云原生平台后端","location":"北京",\
       "salaryMin":20000,"salaryMax":35000}')
JID=$(echo "$JOB" | python3 -c 'import sys,json;print(json.load(sys.stdin)["id"])')
curl -s "$B/jobs?keyword=后端&location=北京" -H "Authorization: Bearer $ST" | head -c 200; echo
APP=$(curl -s -X POST $B/jobs/$JID/apply -H "Authorization: Bearer $ST" \
  -H 'content-type: application/json' -d '{"coverLetter":"你好"}')
AID=$(echo "$APP" | python3 -c 'import sys,json;print(json.load(sys.stdin)["id"])')

# 3) 招聘者推进状态
curl -s -X POST $B/applications/$AID/status -H "Authorization: Bearer $HT" \
  -H 'content-type: application/json' -d '{"status":"interviewing"}'
```

## 测试

集成测试在 `src/tests/`，跑在**真实 PostgreSQL** 上：`#[sqlx::test]` 会为每个用例**单独建库**并自动执行
`./migrations`，用例之间互不污染，结束后自动清理（需要连接账号有 `CREATEDB` 权限，本机 pgdb 的 postgres 满足）。

```bash
cargo test                    # 全部（自动读取 .env 里的 DATABASE_URL）
cargo test auth::             # 只跑鉴权
cargo test complaints::       # 只跑举报审核
cargo test reviewers::        # 只跑审核账号
cargo test review_locks::     # 只跑并行锁定
cargo test stats::            # 只跑平台统计
cargo test complaint_rate::   # 只跑定级 v2（投诉率 × 公司规模）
cargo test jobs_order::       # 只跑职位列表排序
```

| 模块 | 用例要点 |
| --- | --- |
| 鉴权 `tests/auth.rs` | 令牌（缺失/伪造/登出/账号被禁用 一律 401）、注册校验与重复手机号 409、**admin/reviewer 不可自助注册**、登录错误不区分账号是否存在且连续失败 429、角色门禁（求职者不能发职位）、`/users` 越权（非管理员 403、查他人 404、本人可查改自己）、管理员引导的幂等与「绝不提权」 |
| 举报审核 `tests/complaints.rs` | 发起前置条件（必须沟通过 / 证据 ≥20 字 / 仅求职者）、审核权限（仅审核账号或 admin）、**通过后企业投诉次数 +1 且职位列表里该企业降档**、驳回计数与顺序不变、重复审核 409、备注超长 400、可见范围与 `?status=` 过滤 |
| 审核账号 `tests/reviewers.rs` | admin 创建后可用初始密码登录并审核（留痕 `reviewedByName`）、管理接口仅 admin（审核账号自身也 403）、只认 reviewer 角色（对其它角色 404）、禁用即踢下线、改密撤销旧会话、删除清理账号、**多审核账号并行在线各审各的**、同一账号多端登录且登出互不影响、单账号被限流不牵连他人 |
| 并行锁定 `tests/review_locks.rs` | 认领成功并返回锁状态、他人认领 409（报错含持锁人）、本人重复认领=续约（不改写 lockedAt）、锁过期后可被抢占、释放规则（本人可 / 他人 403 / 非管理员 force 403 / 管理员 force 可 / 过期任何人可清理）、他人持锁时审结 409、无锁直审补记接单时间、并发重复审结第二个 409、业务角色不能认领 |
| 平台统计 `tests/stats.rs` | 管理员看全部审核账号而审核账号只看自己、业务角色 403、未认证 401、**及时率口径**（2h 及时 + 48h 超时 → 0.5 且无结论类字段）、公司列表最差在前与质量分公式 `100-已核实*10-待审*2`、求职统计聚合（简历/投递/分档/参与度） |
| 定级 v2 `tests/complaint_rate.rs` | SQL 函数边界与客户端规则逐条对齐（0.5/1.5/3.0% 边界、<50 人退回次数、未申报规模退回次数）、**大公司 20 次投诉排在小公司 3 次之前**、同等级内有规模折算者优先、`companyStaffSize`/`staffSize` 字段透出（未申报为 null）、注册时 `staffSize` 范围校验 |
| 列表排序 `tests/jobs_order.rs` | 分页拼接后仍按等级从优到劣（跨页全局有序）、同级按次数与时间、收藏按等级而非收藏时间、本企业职位按发布时间倒序 |

## 设计要点

### 新增业务模块
1. `migrations/000N_xxx.sql` 写迁移（启动自动执行）；
2. `src/models/xxx.rs` 定义类型并 `#[derive(ToSchema)]`；
3. `src/repositories/xxx.rs` 写 SQL（sqlx 0.9 要求**编译期字面量**，勿用 format! 拼 SQL），错误用 `?` 自动转换；
4. `src/handlers/xxx.rs` 写处理器加 `#[utoipa::path(...)]`；受保护接口在参数里加 `AuthenticatedUser` 与 `security(("bearerAuth" = []))`；
5. `src/openapi.rs` 把 handler 加入 `paths(...)`、类型加入 `schemas(...)`。

### 权限约定
- 处理器层用 `user_repo::require_recruiter` / `require_seeker` / `require_admin` / `require_reviewer` 校验角色；
- 招聘者只能操作**本企业**职位/投递（比对 `company_id`）；
- 求职者只能撤回自己的投递（`withdrawn`），其余状态由招聘者推进；
- 私有简历对其他角色一律 404；
- **平台管理（admin）**：`/reviewers` 全套、`/users` 列表/创建/删除仅 admin；用户查询/修改限**本人或 admin**，查他人按 404 处理（不泄露账号存在性）；
- **审核账号（reviewer）**：仅投诉队列与审核；不能发职位/投递/维护简历，也不能管理账号（`/reviewers` 403）；admin 作为超级角色可代审；
- admin 与 reviewer 都不能自助注册：admin 由服务端 `ADMIN_PHONE`/`ADMIN_PASSWORD` 引导创建（**不会把同手机号的既有账号提权**），reviewer 由 admin 创建；
- 账号被禁用（`is_active=false`）后：既有令牌**立即失效**（401）、重新登录 403；重置审核账号密码同样会撤销其全部会话；
- 多个审核账号可同时在线（令牌按账号多条并存、登录互不踢），某个账号登录失败被限流也不会牵连其它账号（限流键为「手机号|IP」）；
- **并行审理由认领锁保证**：`claim` 用条件 UPDATE（空闲 OR 自己 OR 已过期）抢占，只有一人能成功；`review` 同样要求「未被他人有效锁定」并用条件 UPDATE 兜底并发重复提交；审结会清空锁但保留 `review_started_at`；
- **统计可见范围**：`/stats/review` 对审核账号只返回其本人（看板不互相暴露同事数据）；`/stats/companies` 与 `/stats/seekers` 仅 admin。

### 安全设计
- **SQL 注入**：所有 SQL 均为 sqlx **编译期字面量 + 参数绑定**（`bind`），禁止动态拼接（sqlx 0.9 对非字面量 SQL 直接编译报错）；用户输入只作为绑定值传入；
- **认证**：密码 argon2id；令牌 32 位随机、库中只存 sha256 摘要、可撤销可过期；登录失败按「手机号|IP」**5 次/15 分钟冻结 → 429**（`src/limiter.rs`）；注册/登录错误信息不区分“账号不存在/密码错误”；
- **管理员引导**：`ADMIN_PASSWORD` 只从环境变量读取（不入库、不进日志），引导创建时立即 argon2id 哈希；手机号已被其它角色占用时服务**拒绝启动**而不是提权；
- **输入上限**：所有文本/搜索字段有长度上限（如密码 8-128、职位描述 10000、消息/求职信 2000、keyword 200），数值范围校验（薪资 0~10^7），请求体上限 1 MiB；
- **越权与隐私**：处理器层按角色（`require_seeker` / `require_recruiter` / `require_admin`）+ 资源归属（company_id）+ 本人身份三重校验；私有简历、他人账号对其他角色一律 404（防探测）；
- **CORS**：默认拒绝跨域（同源策略生效）；需要时在 `.env` 配置 `CORS_ALLOWED_ORIGINS=http://localhost:5173,...` 白名单放行；
- **错误脱敏**：5xx 一律不向客户端泄露内部细节，原始错误仅进日志。

### 一致性约定
- 会话唯一化 `(user_lo, user_hi)`；投递唯一约束 `(job_id, seeker_id)`；
- 5xx 统一脱敏、原始错误进日志；`RUST_LOG` 控制级别。

### 数据库注释（COMMENT ON）
- 每个迁移文件末尾都带一组 `COMMENT ON TABLE / COLUMN / CONSTRAINT / FUNCTION`（中文），
  库内可通过 `\d+ 表名`、`obj_description(...)`、`col_description(...)` 随时查阅，不依赖外部文档；
- `migrations/*.sql` 由 `sqlx::migrate!` **编译期内嵌**，改动迁移后需 `cargo build` 才生效。

### 迁移文件修改注意
- sqlx 会对已执行迁移做 checksum 校验：**本地开发库已跑过旧迁移后修改了该文件**，
  下次启动会报 checksum mismatch。重置本地库即可：

```bash
# 重置 pgdb 里的 appdb（如迁移文件已改动导致 checksum mismatch）
docker exec pgdb psql -U postgres -h 127.0.0.1 -c 'DROP DATABASE IF EXISTS appdb;'
docker exec pgdb psql -U postgres -h 127.0.0.1 -c 'CREATE DATABASE appdb OWNER postgres;'
```

## 生产注意
- 远程（SSL）PostgreSQL 需给 sqlx 加 TLS feature（见 README 版本说明）；
- 令牌每请求查库，量大时可换 JWT 或加缓存；搜索量级上来后可加 GIN 全文索引/专用引擎。

---

## 相关仓库

这是三端招聘平台的其中一端，另外两端：

- **后台管理系统（Vue3 + TypeScript）**：[GitHub](https://github.com/CMrookie/meow-star-careers-admin) ｜ [Gitee](https://gitee.com/rookie_c/meow-star-careers-admin)
- **移动端 App（Flutter）**：[GitHub](https://github.com/CMrookie/meow_star_careers_app) ｜ [Gitee](https://gitee.com/rookie_c/meow_star_careers_app)

> 三端共用一套接口契约与角色模型（seeker / recruiter / reviewer / admin），由 OpenAPI 定义。
