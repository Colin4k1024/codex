## 背景
分布式运行时第一步：远程工具执行。exec-server + environment/* 已存在，做集成而非重建。

## 任务
- [ ] exec-server 认证连接与环境注册/bootstrap 集成
- [ ] 远程 FS/进程/沙箱能力发现（capability_discovery）接入编排器
- [ ] thread/start.environments（experimental）实测；不可用则裁剪范围
- [ ] agent 本体仍本地运行（跨机放置见 {{P5-2}}）

## 验收
一个任务在远程环境完成工具执行；能力发现与权限边界测试。

依赖 #90。代码：exec-server/；exec-server-protocol/。

