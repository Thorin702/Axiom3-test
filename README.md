# Axiom3 · 科研智能体工作台

> 以**校验**为核心的科研 Agent 平台 —— 真跑工具、真做计算、出版级产出,**每一条结论都能点回原文溯源**。
> A verification-centered scientific research agent workbench (Tauri · React · Rust).

当前版本 **1.44.13** · macOS 本地测试版(Windows 版待备案后随正式版发布)

---

## 这是什么

Axiom3 是一个面向生物医药科研的**智能体工作台**。你用日常语言提出课题,它像一位严谨的研究员那样:检索**真实文献与科学数据库**、在**科学计算内核**里跑真实分析、绘制**出版级图表**,最后交付一份分节、带引用、可复核的研究简报——而不是凭记忆编一段听起来很像的话。

与"聊天式"AI 的根本区别:**不编造**。关键结论必须挂上 `[Sn]` 角标,点击即回溯到检索到的原始文献/数据记录;无来源的数字要么补检索坐实,要么显式标注为估计;悬空/编造的引用会被自动拦截。

---

## 核心能力(数字均可在源码中复核)

| 维度 | 数字 | 说明 |
|---|---|---|
| **智能体模式** | **3** | 分析 / 研究 / 代码;研究模式支持**多智能体**(委派子团队 · 独立跨模型审查员 · 交叉验证) |
| **核心工具** | **24** | 计划 · 文件读写 · 检索 · Python 科学计算内核 · 委派 · 后台作业 · 断点续跑 · 中途问询 … |
| **数据接口** | **30 连接器 → 20+ 数据库** | 直连 UniProt · PDB · PubChem · ChEMBL · NCBI/Entrez · KEGG · STRING · AlphaFold · Open Targets · gnomAD · GWAS Catalog · ClinicalTrials.gov … |
| **专家技能** | **15** | 方法学技能卡(可经 `~/.axiom3/skills` 扩展),含 3 个真实图引擎 PyMOL / RDKit / dna_features_viewer |
| **图表渲染器** | **28** | 原生 SVG 科学图:火山图 · 曼哈顿图 · KM 生存曲线 · 森林图 · PRISMA · 热图 · 网络图 · 系统发育树 … |
| **大模型接入** | **7** | Anthropic · OpenAI · Gemini · DeepSeek · 通义千问 · 智谱 · SCNet 超算互联网(主力 **GLM-5.2**) |

---

## 特性

- **双模式** —— 分析(本地数据、有预算、快)与研究(联网优先、无预算、可拆分多智能体、深度综述)。
- **真跑,不假装** —— 每步都执行真实工具:文献检索、数据库查询、Python 内核里的统计与建模、绘图,过程可展开审计。
- **可溯源** —— 报告中的 `[Sn]` 角标可点击回溯到原始文献/数据记录;跨子智能体的来源自动重编号合并。
- **出版级产出** —— 顶刊图/写作规范内建;28 个原生图表渲染器 + PyMOL/RDKit 图引擎;一键导出 **Word / PDF / HTML**。
- **校验为核心** —— 独立的 `axiom-core` 验证引擎(声明模型、引用/蕴含校验、确定性检查器),编排器与验证器解耦。
- **不丢产出** —— 运行崩溃/关窗守卫 + 磁盘持久化 + 中断续跑;产出物库磁盘备份。
- **隐私优先** —— 密钥进 OS 钥匙串;遥测**由类型强制**只发匿名交互元数据(枚举/数字/布尔,课题/数据结构上进不了事件);可一键关闭。

---

## 技术架构

**桌面端**:Tauri v2 + React 19 + TypeScript(`axiom3-desktop/`)。
**核心引擎**:Rust workspace,8 个 crate:

| Crate | 职责 |
|---|---|
| `axiom-core` | 校验核心:声明模型、引用/蕴含校验、确定性检查器、标定(编排器 `catch_unwind` 隔离单个验证器崩溃) |
| `axiom-orchestrate` | 动态多智能体编排引擎(workflow + coding 循环)、科学计算内核、工具调度、收尾协议与门禁 |
| `axiom-runtime` | Agent 运行时(基于 Rig)+ 各家模型适配器 |
| `axiom-providers` | 模型接入层:规范消息模型、能力矩阵、按角色路由(跨厂商独立性) |
| `axiom-store` | 存储:SQLite(SQLCipher)+ 哈希链事件日志 |
| `axiom-vector` | LanceDB 向量索引 |
| `axiom-secrets` | 密钥存储:macOS Keychain / Windows Credential Manager |
| `axiom-ingest` | 文档抽取(txt / md / docx / pdf) |

**数据接口**:`crates/axiom-orchestrate/src/scidb.py` —— 30 个带来源标记的科学数据库连接器(`import scidb` 即用)。

---

## 构建与运行

前置:Node(pnpm)、Rust 工具链、Tauri v2 依赖([tauri.app 环境要求](https://tauri.app))。

```bash
cd axiom3-desktop
pnpm install

# 开发(热重载)
pnpm tauri dev

# 打包本地测试版(产出 .app / .dmg)
pnpm tauri build
```

模型密钥:在 App「设置 → 模型连接」里填写,或用环境变量(如 `SCNET_API_KEY` / `ZHIPU_API_KEY` / `DEEPSEEK_API_KEY` …);密钥落 OS 钥匙串,不进 webview。

Rust 侧测试:

```bash
cargo test -p axiom-orchestrate --lib      # 编排/收尾协议(含真实 GLM 自检 harness)
cargo test --workspace
```

---

## 项目结构

```
科研平台/
├── axiom3-desktop/          # Tauri v2 + React 19 桌面端
│   ├── src/                 #   前端(含 28 个原生图表渲染器 src/components/artifact/)
│   └── src-tauri/           #   Rust 后端命令(工作台/遥测/账号/产出物库/密钥)
├── crates/                  # Rust workspace 核心引擎(8 个 crate)
│   └── axiom-orchestrate/
│       ├── src/scidb.py     #   30 个科学数据库连接器
│       ├── src/skills.rs    #   15 张方法学技能卡
│       └── examples/        #   live 测试 harness(含 finalize_selftest 真实模型自检)
└── *.md                     # 设计文档(架构/选型/护城河/威胁模型)
```

---

## 隐私与安全

- **数据留本机**:课题、数据、报告绝不上传;遥测仅发匿名交互元数据(启动/任务完成的模式·状态·时长等有界枚举),且由 Rust 类型系统强制——自由文本字段不存在,可一键关闭。
- **密钥**:BYO API key 与数据库密钥存 OS 钥匙串,不进前端。
- **验证优先**:确定性检查器 + 引用校验独立于生成模型;审计事件哈希链留痕。

---

## 状态

内测阶段,持续迭代中(macOS 本地测试版)。正式双端版本待 ICP 备案完成后一次性发布。

<sub>© Axiom3 · 主力模型 GLM-5.2(SCNet 超算互联网)</sub>
