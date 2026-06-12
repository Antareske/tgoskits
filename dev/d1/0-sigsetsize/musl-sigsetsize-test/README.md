# musl-sigsetsize-test

这个测例把 `www/1-sigsetsize/benchmark` 中 `1-` 前缀测例的报错节点改为 **libc 标准接口**调用，
只观察标准库包装层行为，不直接发起 raw syscall。

覆盖节点：

- `epoll_pwait(sigmask != NULL)`
- `pselect(sigmask != NULL)`
- `ppoll(sigmask != NULL)`
- `signalfd(sigmask != NULL)`

说明：`musl` 不提供 `epoll_pwait2` wrapper，因此该接口不在本测例覆盖范围。

预期：上述 libc 调用不应因 `sigsetsize` 不匹配返回 `EINVAL`。

## Build

```bash
cc -O2 -Wall -Wextra -std=c11 main.c -o musl-sigsetsize-test
```

如果要在 musl 下验证，可改用：

```bash
musl-gcc -O2 -Wall -Wextra -std=c11 main.c -o musl-sigsetsize-test
```

## Run

```bash
./musl-sigsetsize-test
```
