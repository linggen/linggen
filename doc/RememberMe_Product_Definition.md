# RememberMe — 产品定义文档

## 🎯 一句话简介  
**RememberMe = 开发者的个人 AI 记忆层（Personal AI Brain）**  
让 AI 不再忘记你，不再忘记你的项目、风格、历史与偏好。  
无论你使用 Cursor、VS Code、ChatGPT、Claude 还是本地 LLM，RememberMe 都能自动增强你的提问，让 AI 输出更精准、更懂你、减少重复解释。

---

## 🧭 产品定位  
RememberMe 不是新的 AI IDE，不是 Cursor 替代品，不是另一个 Chat。  
它是：

### ⭐ AI 的“前置记忆 + 意图理解 + 智能提问中间层”  
- Cursor → 提供智能编辑  
- ChatGPT/Claude → 提供强力推理  
- **RememberMe → 提供长期记忆、偏好学习、项目理解、自动补全智能提问**

它站在所有 AI 工具之前，成为开发者的“AI 影子大脑”。

---

## 🧩 解决的问题（用户痛点）

### 1. AI 总是忘记：  
用户需要重复解释：
- 项目架构  
- 代码模块关系  
- 风格偏好  
- 测试写法  
- 错误上下文  
这是最大疲劳来源。

### 2. 用户不知道怎么问  
开发者常常只能说：“帮我修一下”“解释下”。  
AI 不理解真正意图 → 输出差。

### 3. Prompt 太长，手写很烦  
用户不想手工复制一堆上下文（diagnostics、类型、代码片段、历史讨论…）。

### 4. Cursor/ChatGPT 没有长期记忆  
关闭项目、换会话、换工具 → 全部归零。

### 5. AI 输出风格不一致  
用户喜欢 bullet points？喜欢 concise？喜欢 AAA？  
AI 永远记不住。

RememberMe 全部解决。

---

## 🧠 核心价值  
### ⭐ 帮用户构建：“AI 长期记忆 + 用户风格 + 项目理解”  
### ⭐ 不改变用户习惯，自动增强提问  
### ⭐ 减少解释成本，让 AI 越用越懂你

---

## 🚀 核心能力

## 1. Intent Engine（意图识别）

自动识别用户一句话背后的真实目的：

- fix bug  
- explain code  
- summarize module  
- write tests  
- refactor  
- debug  
- analyze error  
- generate doc  
- learn pattern  

意图 → 进入 MemoryInjector。

---

## 2. Memory Engine（记忆层）

跨工具、跨模型、跨项目的统一记忆库：

### 🧩 Project Memory  
记住：
- 模块结构  
- 项目架构  
- 类型关系  
- 历史 bug  
- 历史总结  
- 项目惯用模式  

### 🧩 User Preference Memory  
记住：
- 用户解释风格  
- 文档风格  
- 测试风格  
- 语言偏好  
- 习惯（简短/详细）  
- 常用 prompt 结构  

### 🧩 History Memory（长程记忆）  
记住用户在多个应用里的所有交互（用户允许时）：
- Cursor 中的问题  
- ChatGPT 的对话  
- Claude 的上下文  
- VS Code 里的命令  

**形成用户的“AI 人格模型（Persona Model）”。**

---

## 3. Prompt Completion（自动补全提问）

用户只说一句：

> “帮我解释这个函数”

补全后（用户看不到）：

- 注入选中代码  
- 注入相关类型  
- 注入历史讨论  
- 注入该模块记忆  
- 注入用户偏好（bullet, concise）  
- 注入项目架构  
- 注入测试风格  
- 注入先前错误分析  

生成一句更精准的提问：

> “请用简短 bullet 解释 login 函数的逻辑、数据流、失败条件，并结合 auth 模块的记忆分析潜在边界问题。”

最终用户看到的仍是一句简短话，但输出质量倍增。

---

## 4. 输入层（Input Layer）

### ⭐ 不替代 Cursor  
### ⭐ 不创建聊天 UI  
### ⭐ 只创建一个“轻量输入面板（Spotlight风格）”

按快捷键 → 输入一句 → RememberMe 增强 → 自动贴入 Cursor → 用户按 Enter。

体验流：

1. ⌘ + Shift + E → 打开输入框  
2. 用户输入一句话  
3. RememberMe 补全 + 注入 Memory  
4. 自动聚焦 Cursor Chat  
5. 自动粘贴  
6. 用户按 Enter  
7. Cursor 输出更聪明的回应  

**Cursor 不需要被 Hook，也不会禁用这个插件。**

---

## 🧩 技术架构

```
+----------------------+
|    User Input Layer  |  (浮动输入框/快捷键)
+----------+-----------+
           |
           v
+----------------------+       +----------------+
|   Intent Engine      | --->  |  Memory Store |
+----------+-----------+       |  (LanceDB)    |
           |                   +-------+--------+
           v                           ^
+----------------------+               |
|   Memory Injector    | --------------+
+----------+-----------+
           |
           v
+----------------------+
| Prompt Completion    |
|  (Local LLM / Mini)  |
+----------+-----------+
           |
           v
+----------------------+
|  Delivery to Cursor  |
+----------------------+
```

---

## ⚙️ 可行性确认  
全部能力在：

- VS Code API  
- 本地 LLM  
- 剪切板  
- 窗口 focus  
- 自己的输入层  

实现，不需要 Hook Cursor，不侵入安全沙盒。  
**Cursor 不可能封杀。**

---

## 🔐 隐私和本地化  
RememberMe 完全可以做到：

- 本地运行  
- 本地存储  
- 不上传任何代码  
- 用户完全控制记忆  
- 离线小模型推理（Qwen/Phi）  

这是 Cursor、ChatGPT 无法提供的能力。

---

## 🏆 产品的护城河（为什么 Cursor 抄不走）

### 1. 跨工具记忆（Cursor 永远做不了）  
### 2. 用户长期人格模型  
### 3. 跨项目知识图谱  
### 4. 本地隐私合规  
### 5. 中间件层位置（不是应用层）  
### 6. 多模型适配（Cursor ≠ 平台）  
### 7. 记忆资产是用户的成长，不是一个功能  

你不是一个“功能”，你是用户的“AI 大脑”。

---

## 🌱 MVP 范围（可 2–3 周完成）

### 1. Floating Input Layer (Hotkey trigger)  
### 2. Local mini model 做 Prompt Completion  
### 3. LanceDB 做项目 + 用户记忆  
### 4. 自动构建增强 prompt  
### 5. 自动复制 & 聚焦 Cursor  
### 6. 用户按 Enter → 完整体验形成

---

## 📦 附录：用户使用流程（最终体验）

> 我按一个快捷键 → 输入一句话 → Cursor 的 AI 突然像懂我一样回答了。

用户感知：

- 我不用解释那么多  
- AI 能记住我喜欢怎样解释  
- AI 知道我项目结构  
- AI 不再重复问  
- AI 输出风格终于一致  
- 使用成本变得“惬意”  

---

## 📘 总结  
RememberMe 不是聊天工具、不是 IDE 插件，它是：

> **AI 时代每个开发者的个人长期记忆层。  
让 AI 越用越懂你，成为你的影子大脑。**  
