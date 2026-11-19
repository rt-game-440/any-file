# 领域文档（Domain Docs）

规范工程技能在探索代码库时如何消费本仓库的领域文档。

## 探索代码前先阅读

- 仓库根目录的 **`GLOSSARY.md`**，或
- 若根目录存在 **`GLOSSARY-MAP.md`**：它指向各个上下文独立的 `GLOSSARY.md`，阅读与当前主题相关的条目。
- **`docs/adr/`**：阅读涉及当前工作区域的架构决策记录（ADR）。在多上下文仓库中，还需检查 `src/<context>/docs/adr/` 中的上下文级决策。

如果上述文件尚不存在，**静默继续即可**。不要报错缺失，也不要主动建议提前创建。`/domain-modeling` 技能（通过 `/grill-with-docs` 和 `/improve-codebase-architecture` 触发）会在术语或架构决策真正敲定时按需懒创建这些文件。

## 文件结构

单上下文仓库（single-context，本仓库采用此布局）：

```
/
├── GLOSSARY.md
├── docs/adr/
│   ├── 0001-event-sourced-orders.md
│   └── 0002-postgres-for-write-model.md
└── src/
```

多上下文仓库（multi-context，以根目录存在 `GLOSSARY-MAP.md` 为标志）：

```
/
├── GLOSSARY-MAP.md
├── docs/adr/                          ← 系统级架构决策
└── src/
    ├── ordering/
    │   ├── GLOSSARY.md
    │   └── docs/adr/                  ← 上下文级架构决策
    └── billing/
        ├── GLOSSARY.md
        └── docs/adr/
```

## 遵循术语表词汇

当输出中涉及领域概念（Issue 标题、重构提案、假设、测试名称等）时，统一使用 `GLOSSARY.md` 中定义的术语，不要漂移到术语表明确避免的同义词。

若需要的概念尚未收录在术语表中，这是一个信号：要么你引入了项目并不使用的说法（应重新考虑），要么存在真实的术语缺口（记录下来供 `/domain-modeling` 补充）。

## 显式标出 ADR 冲突

如果输出与现有 ADR 相矛盾，请明确指出，不要静默覆盖：

> _与 ADR-0007（事件溯源订单）冲突，但值得重新讨论，因为……_
