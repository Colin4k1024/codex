# P1-1 / #73 结论：能力包结构与 manifest（digest/许可/版本绑定）

日期：2026-11-17 · 分支：feat/p1-1-pack-manifest · 零引擎源码改动（产品侧 superagent/）

## 交付

- `superagent/pack/`：roles/ skills/ workflows/ templates/ 四目录 + LICENSES.json + manifest.json。
  目录内为结构规范（README）与版本绑定模板（templates/run-record.json）；
  角色内容=#74、team-delivery 技能=#75、workflow schema=#78 后续交付（依赖顺序）。
- `superagent/tools/pack.py`：确定性 digest 导出/校验工具。
  规范化=pack 内全部常规文件（除 manifest.json 自身），相对 POSIX 路径字典序，
  逐文件 sha256，digest=sha256("<path>\t<filehash>\n" 连接)。符号链接/非常规文件 fail-closed 拒绝。
- 许可完整性：LICENSES.json 分组（id/source/paths glob）必须覆盖每个文件，缺失即校验失败。
  当前全部内容为仓库原创（Apache-2.0），third_party=[]。
- 版本绑定：run 记录固定 bundle digest（templates/run-record.json 示例）；
  `verify --pinned <digest>` 不符即显式失败（"never silently substitute"），
  恢复/重放必须用原 pin 的包；升级=新 run 新 pin，旧 run 不受影响。
  吸收 HARNESS_INTEGRATION §5 bundle/digest 原则（该路线已作废，原则继承）。

## 验收对照（全部实测，model-free）

| 验收 | 结果 | 证据 |
|---|---|---|
| 干净环境 manifest 校验通过 | PASS：无 git 元数据的全新临时目录拷贝 verify OK | pack-ci.log（clean copy verify）、clean-verify.txt |
| digest 可复现 | PASS：CI 重导出比对 digest 一致；两次导出字节一致；文件创建顺序无关 | pack-ci.log（re-export comparison PASS）、test-output.log test_02/03/04 |
| 许可清单完整 | PASS：LICENSES.json 覆盖全部 6 文件；覆盖缺失/文件缺失均被拒绝（负面测试） | test-output.log test_09/10 |

测试：11/11 PASS（test_pack.py，-W error::ResourceWarning 下亦通过）：
committed manifest 校验、导出确定性、创建顺序无关、干净环境、篡改检测、
schema 强制、pinned 绑定（正/负）、符号链接拒绝、许可覆盖、许可文件缺失、file_count 不符。

当前 manifest：name=superagent-meta-agent v0.1.0，
digest=5b26dcb803642b472a2d91d44cc0cf745319b30fbc61cd78e6a9cf5f58c204a7，file_count=6，
source_commit=697de6498（导出基线；digest 计算不含 source_commit，重导出比对不受其影响）。

## 依赖与合并顺序

issue 声明依赖 #72（P0-9 接口决策）。本实现不依赖 #72 剩余探针结果
（pack 结构与 digest 规则与 turn/start permissions/environments 裁决正交），
故提前并行开发；**合并仍按依赖顺序：#72 关闭后再合入本 PR**。
若 #72 定稿产生与本结构冲突的决策，按决策整改后再审。

## 复现

```
bash superagent/tests/pack-ci.sh          # CI 门禁：重导出比对 + 11 测试
python3 superagent/tools/pack.py verify --pack superagent/pack
python3 superagent/tools/pack.py verify --pack superagent/pack --pinned <digest>
```
## 审查记录

grok r1 APPROVE（grok-review.json），2 项非阻塞 minor 如实记录：
①manifest.source_commit 为导出基线（697de6498），属溯源元数据、不参与 digest；
②manifest.json 自身不在许可覆盖检查内（生成元数据，LICENSES.json 的 ** 模式已声明其许可）。
合并按依赖顺序：#72 关闭后执行。
