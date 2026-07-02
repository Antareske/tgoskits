# nginx large file sendfile 间歇性失败记录

## 背景

这份记录对应 `apps/starry/nginx` 的默认 smoke 中 `large file sendfile` 步骤。它属于默认 CI 入口的一部分，不是 phase/debug 专项脚本。

在此前排查里，默认 smoke 已经能覆盖：

- `GET /`
- `GET missing returns 404`
- `HEAD /small.txt`
- `keepalive two requests`
- `nginx master config test`
- `master GET /`
- `master reload`

真正出问题的是 `sendfile on` 的大文件下载步骤。

## 发现过程

### 第一次暴露

在默认 smoke 里，`large file sendfile` 偶发失败，curl 报：

```text
curl: (18) end of response with 528992 bytes missing
```

这类错误的含义是：HTTP 响应头已经收到，但 body 没有完整传完，连接提前结束。

### 与其它步骤的对比

同一次执行里，前面的步骤都正常：

```text
NGINX_APP_STEP_PASS: GET /
NGINX_APP_STEP_PASS: GET missing returns 404
NGINX_APP_STEP_PASS: HEAD /small.txt
NGINX_APP_STEP_PASS: keepalive two requests
NGINX_APP_STEP_PASS: master reload
NGINX_APP_STEP_PASS: master quit
```

失败严格收敛在：

```text
NGINX_APP_LOG: BEGIN large file sendfile
curl: (18) end of response with 528992 bytes missing
NGINX_APP_STEP_FAIL: large file sendfile
```

### 后续再次复测

在再次复测的 x86_64 默认 smoke 中，仍然复现了同类错误，只是缺失字节数变化了：

```text
NGINX_APP_LOG: BEGIN large file sendfile
curl: (18) end of response with 528992 bytes missing
NGINX_APP_STEP_FAIL: large file sendfile
```

另一轮 x86_64 复测中，缺失字节数又变成：

```text
curl: (18) end of response with 138128 bytes missing
```

这说明它不是固定偏移的静态截断，更像是间歇性时序问题。

### 默认 smoke 的 loongarch64/riscv64 对照

在默认 smoke 中：

- `riscv64` 曾经通过完整 smoke
- `loongarch64` 在某些轮次也能走到 sendfile 之后的深测，但更容易受 guest 资源和镜像安装阶段影响

因此这不是单架构固定坏掉，更像是不同架构在不同负载下把同一类问题放大出来。

## 原始输出摘录

### 默认 smoke 失败 1

```text
NGINX_APP_LOG: BEGIN prepare packages
NGINX_APK_MIRROR_TRY: https://mirrors.tuna.tsinghua.edu.cn/alpine/latest-stable attempt=1
NGINX_APK_MIRROR_OK: https://mirrors.tuna.tsinghua.edu.cn/alpine/latest-stable
NGINX_APP_STEP_PASS: prepare packages
...
NGINX_APP_LOG: BEGIN nginx sendfile config test
nginx: the configuration file /tmp/nginx-tests/conf/sendfile.conf syntax is ok
nginx: configuration file /tmp/nginx-tests/conf/sendfile.conf test is successful
NGINX_APP_STEP_PASS: nginx sendfile config test
NGINX_APP_LOG: BEGIN start nginx sendfile
NGINX_APP_STEP_PASS: start nginx sendfile
NGINX_APP_LOG: BEGIN large file sendfile
curl: (18) end of response with 528992 bytes missing
NGINX_APP_STEP_FAIL: large file sendfile
```

### 默认 smoke 失败 2

```text
NGINX_APP_LOG: BEGIN large file sendfile
curl: (18) end of response with 138128 bytes missing
NGINX_APP_STEP_FAIL: large file sendfile
```

### debug 首轮失败、后续成功

`qemu-x86_64-sendfile-on-debug.toml` 对应脚本里，第一次大文件请求失败，但后续第 2 到第 5 次成功：

```text
NGINX_PHASE42_DEBUG_LOG: probe=1 curl_fail size=910448
NGINX_PHASE42_DEBUG_LOG: probe=2 curl_ok size=1048576
NGINX_PHASE42_DEBUG_LOG: probe=3 curl_ok size=1048576
NGINX_PHASE42_DEBUG_LOG: probe=4 curl_ok size=1048576
NGINX_PHASE42_DEBUG_LOG: probe=5 curl_ok size=1048576
NGINX_PHASE42_DEBUG_DONE
```

这组输出非常关键：它说明问题不是恒定复现，而是首轮/初次传输更容易出错。

### debug 完全成功的一次

另一次 debug 探针是完全成功的：

```text
NGINX_PHASE42_DEBUG_LOG: probe=1 curl_ok size=1048576
NGINX_PHASE42_DEBUG_LOG: probe=2 curl_ok size=1048576
NGINX_PHASE42_DEBUG_LOG: probe=3 curl_ok size=1048576
NGINX_PHASE42_DEBUG_LOG: probe=4 curl_ok size=1048576
NGINX_PHASE42_DEBUG_LOG: probe=5 curl_ok size=1048576
NGINX_PHASE42_DEBUG_DONE
```

## 静态判断

- 失败点是 `sendfile on` 的大文件传输，不是基础 HTTP 交互
- `curl (18)` 表示 body 不完整，属于传输被截断
- 现象是间歇性的，且 debug 里表现为“首轮失败、后续成功”或“全部成功”两种状态
- 宿主机后台压测/竞争可能放大问题，但不是唯一可疑点

## 相关配置

- 默认 smoke：`apps/starry/nginx/smoke/nginx-smoke-tests.sh`
- sendfile 专项脚本：`apps/starry/nginx/phase/nginx-4-2-sendfile-on-tests.sh`
- debug 定位脚本：`apps/starry/nginx/debug/nginx-4-2-sendfile-on-debug.sh`
- debug QEMU：`apps/starry/nginx/qemu/debug/qemu-x86_64-sendfile-on-debug.toml`

## 影响范围

- 影响默认 smoke 的 `large file sendfile`
- 不影响 `GET /`、`HEAD /small.txt`、keepalive、master lifecycle 这些基础项的结论
- 不建议把大文件 sendfile 结果直接当成全局 apps CI 的唯一健康度指标

## 当前判断

更像是：

- `sendfile on + 大文件 + event/socket 时序` 的间歇性问题
- 在宿主机负载上升时更容易暴露
- 在 debug 模式下可复现为“偶发首轮失败，后续成功”

## 后续观察点

- 是否与宿主机后台压测并发有关
- 是否只在首次大文件传输发生
- 是否在更低系统负载下明显缓解
