# 与AI合作对starry OS改进的开发方案和进展情况

# 三种改进starryos的开发方案

**注：修复bug/功能添加的过程，不是让AI代替你的过程，而是你提升自己的过程。所以，需要你向AI学习，包括OS的设计实现思路、OS的调试思路。到你提升到一定程度，你将以架构师的角度来指导AI完成OS的研发**

**注：小步快跑很重要！在提交PR前，先下载/merge最新的tgoskits 中dev分支的最新代码，可能dev分支已经有了新的代码，覆盖/冲突/重现你的PR中的部分内容。如出现这种情况，通过大模型分析最新的dev分支的代码，并更新PR，再提交。**

**注：合理的commit内容撰写参考：例如 https://github.com/rcore-os/tgoskits/pull/219 （来自 @Tempest 赵光烨），给出了bug 描述，根本原因分析，fix描述，测试plan, 修的戴内核代码和测例简要说明。但这个例子提供较早，没有提交到dev分支中。**

## 开发方案的前期环境支持
- 能在本地CI，本地可建立与github上一致的docker开发与测试环境
- 对单点功能修复的测试支持：支持在仓库中添加可发现某个syscall相关或某内核某功能点有bug/实现缺失等情况的C（已实现）和Rust（待实现）源码级用户态测例用例程序（规模），CI可在4个硬件环境+alpine linux中自动测试
- 支持完整linux app的测试支持：支持在仓库中添加脚本，可通过 apk add (alpine linux环境)或apt install (debian linux环境)来安装linux app，CI可在4个硬件环境+alpine/debian linux中自动测试


## 一、方案一：以1~2个syscall为引导的starryos改进
注：目前在支持基于musl-libc的alpine和基于glibc的debian两个linux distributions。
助教微信id：“Mr Graveyard”

进行方案一的项目实习，是参加方案二和方案三的基础。请联系助教了解细节。

# 当前系统调用完成情况统计
测例完成情况统计表：https://docs.qq.com/sheet/DRnhPVk13SFdsemdq?tab=BB08J2 各位注意认领后再开始工作
系统调用优先级列表：https://github.com/rcore-os/linux-compatible-testsuit/blob/dev/docs/starryos_syscall_priority.md

1. 让大模型分析syscall的通用性和重要性，并进行排序，形成文档：https://github.com/rcore-os/linux-compatible-testsuit/blob/dev/docs/syscall_priority.md
2. 选择一个syscall，让大模型根据  `man 2 syscall-nanme`  分析这个syscall的语义，并生成用户态测例，能够分析在linux在正确执行或非正确执行该syscall的返回情况，并通过 `assert`判断各种返回情况的正确语义。
3. 在本地开发，在starryos中添加并测试这个源码级的用户态测试用例（参考 https://github.com/rcore-os/tgoskits/pull/235  PR： feat(starry): add C/Rust userspace test case infrastructure ），一般位于 https://github.com/rcore-os/tgoskits/tree/dev/test-suit/starryos/normal/ 目录下，如果某 assert失败，则大概率starryos有bug，就与大模型合作进行修复。
4. 修复完毕并自己review且理解并掌握错误的原因和修改的原因后，提交PR（包含源码级测例和对内核的修改）到tgoskits的dev分支。
5. reviewer 与大模型合作查看PR后， merge PR；如果有问题，在PR中给出改进意见，请提交者改进。

## 二、方案二：以linux app为引导的starryos改进
注：目前在支持基于musl-libc的alpine和基于glibc的debian两个linux distributions。
alpine的助教微信id：“AsakuraMizu”
debian的助教微信id：“fluove_top”

进行方案二的项目实习，是参加方案三的基础。请联系助教了解细节。

1. 按linux app的基础性、简单/复杂度与个人兴趣等角度（可参考app推荐： https://github.com/rcore-os/tgoskits/discussions/228#discussioncomment-16587718 ）选择某starryos还不支持的linux app
2. 在本地开发，在starryos中通过添加配置文件的方式添加并测试这linux app（可参考 https://github.com/rcore-os/tgoskits/tree/dev/test-suit/starryos/stress/stress-ng-0 中测试stress-ng的4个配置文件）。
3. 在测试过程中，与大模型合作，让大模型每修复一个bug，最好能生成一个针对这个bug的单一的源码级用户态测试用例文件。如果可能存在较为复杂的触发逻辑，难以生成相关的测例，也需要详述该bug根本原因和触发逻辑。可参考改进方案一，添加到starros的源码测试集（在 https://github.com/rcore-os/tgoskits/tree/dev/test-suit/starryos/normal/ 目录下）中，并提交针对这个bug的PR（包含源码级测例和对内核的修改）到tgoskits的dev分支。
4. 重复步骤3 ，进行一个一系列的bug修改和功能改进/添加，直到linux app能正常运行。提交针对linux app运行测试的PR（包含 linux app测试的配置文件）到tgoskits的dev分支。
5. 子课题挑战：1. 多核并行支持与优化；2. backtrace的支持与优化；3. 基于ebpf的内核可观测性能力增强；4. wayland的图形支持；

## 三、方案三：以移动机器人为引导的全栈starryos改进
注：目前支持sg2002/rk3588，兼容alpine linux为目标
sg2002机器人的助教微信id：“master-ajax”
rt3588机器人的助教微信id：“zhourui747”

starry for sg2002机器人的当前进展：https://github.com/orgs/Starry-OS/discussions/24

当前还在快速迭代开发中，starry on sg2002/rs3588 机器人已经完成了基本功能。想参与方案三的同学，请先在方案一和方案二上进行前期项目实践，打好starry的相关基础，然后联系助教，逐步过度到方案三。

注：子课题挑战：1. 基于NPU/TPU的AI全栈性能优化


# 基于“一、以1~2个syscall为引导的starryos改进方案”进行PR提交的参考示例
SZH提供了 https://github.com/rcore-os/tgoskits/pull/235 的PR是一个提交fixbug的PR的参考示例，应该包含两部分：
1.  C源码测例和相关文件，位于 https://github.com/rcore-os/tgoskits/tree/dev/test-suit/starryos/normal/helloworld/c 目录下。
2. 对arceos/starry内核组件的修改。由于这只是一个例子，所以这里没有。
   另外，还提供了 https://github.com/rcore-os/tgoskits/blob/dev/test-suit/starryos/GUIDE.md 说明了 添加 StarryOS QEMU 测试用例的详细内容。

# 基于docker container进行开发的参考示例
这是一个提交PR的例子( https://github.com/rcore-os/tgoskits/pull/252 )，整个过程是基于 https://github.com/rcore-os/tgoskits/pkgs/container/tgoskits-container 中的image，在container中完成。大致执行命令如下：
```
#用自己的GitHub 的 Personal Access Token (PAT) 登录docker
echo ghp_Ig6WW580bNNu1CuD4Gqc4mVxcM15n41YC0XXXX | docker login ghcr.io -u chyyuu --password-stdin

#下载docker 镜像
docker pull ghcr.io/rcore-os/tgoskits-container:latest

#进入docker container中进行开发 $(pwd)可以是包含本地的tgoskits的目录
docker run -it --name tgoskits
-v "$(pwd)":/workspace
-w /workspace
ghcr.io/rcore-os/tgoskits-container:latest /bin/bash

#在 docker中进一步安装相关工具 claude code... ，能够进行AI编程，git使用的等，比如:
apt install vim openssh-server openssh-client ...
curl -fsSL https://claude.ai/install.sh | bash //安装claude clode CLI
....

# 进行开发，添加 test_pipe2 测试用例，测试通过
cargo starry test qemu -t riscv64 -c test_pipe2
cargo starry test qemu -t aarch64 -c test_pipe2
cargo starry test qemu -t x86_64 -c test_pipe2
cargo starry test qemu -t loongarch64 -c test_pipe2

# 提交代码
git push chy dev:dev

在自己的repo中，提交PR
```

# 方案一：参与人员初步安排

- 生成新的Linux兼容的测例
  相关新增测例存放在本仓库的dev分支的test文件夹中
  目前进展：已经跑通流程，需要继续按照docs里面的优先级文档增加新的测试
- 使用生成的Linux兼容测例，测试starry OS并修复bug，提交pr
  使用dev分支的run_all_tests脚本进行运行测例，使用ai查看运行结果并修复并提交pr
  目前进展：已经跑通流程，可以用AI较为自动的测试，修复，提交pr（可能需要少量人工介入）
  目前相关人员：朱懿、王乙凡、戴骏翔、徐子航、郭士尧、李璜华、王铮

- merge相关pr
  建议探索方向：使用ai来review相关pr。使用cli工具配合AI快速总结pr的修复内容，审查pr的编码规范等问题。
  目前进展：尚未得到流程跑通的反馈，但已经可以看到ai reviewer的参与
  目前相关人员：王铮

- step4: 添加已经通过的测例到tgoskits的ci测例集合中，不断迭代完善tgoskit的ci测试范围
  目前进展：已经跑通流程
  目前相关人员：邵志航、周睿、赵长收


# 当日进展
为让大家相互了解，避免重复，冲突等，请：
如当天有进展，请当天及时在下面写出（下面是例子）：
2026.04.16
xxx：今天建立了..., 修复了....

# 周一（2026.4.20）计划
- 对于syscall_priority.md文件中指定的一百多个syscall是否在https://github.com/rcore-os/tgoskits/tree/dev/test-suit/starryos/normal 中已经存在测例及是否通过情况进行搜集并统计。
  相关人员：戴骏翔
  预计完成时间：周一上午
- 根据上一步的统计情况，持续跟新这个统计表，并走完从写测例、测试测例发现bug，修复bug，提交PR(测例源码+修复代码)这4步构成的整个流程。
  相关人员：戴骏翔、朱懿、王乙凡、徐子航、郭士尧、李璜华、王铮
- 准备好三个架构的debian rootfs支持
  相关人员：赵长收

