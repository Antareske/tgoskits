WiFi的主要业务是控制业务和数据业务：aicdevice（状态机），sdiocard（sdio卡操作接口）

向下：
与芯片通讯：需要SDIO卡协议的封装：sdmmc（协议实现，SDIOHost控制器）

向上：
向上第一层：rdif 适配层：owner
向上第二层：ax-net 网络运行时和网络栈

上层这样使用 AIC 驱动：

  1. 取得设备：OS Glue 创建 AicRdifDevice 并登记。ax-net 接收它，调用 into_parts，取得通用的网络队列、Wi-Fi 控制接口、启动接口和中断接口。

  2. 启动设备：ax-net 的队列线程调用启动接口。启动接口使用 AicOwner 完成 SDIO 卡识别、固件和芯片初始化。完成后，同一个 owner 交给运行期接口持有。

  3. 日常使用：网络栈通过 TX/RX 队列收发 Ethernet 帧，通过 Wi-Fi 控制接口发起连接等操作。队列线程根据队列工作、中断或定时唤醒推进 owner；owner 再使用 AicDevice 和 SdioCard 与芯片通信。











=============================================






读写芯片寄存器通常使用SDIO的CMD 52，读写FIFO通常使用CMD 53；共用相同的完成通知和设备中断，
第8连控制和数据共用SDIO的Function 1，DC固件命令走Function 2，普通数据走Function 1，双通道

SDIO Card提供SDIO卡协议层的操作接口
例如初始化卡、启动方Function、设置块大小、读写单个字节、通过DMA读写数据
这些接口通常分两步使用，先提交操作取得该操作的请求对象，等控制器中断到来或需要继续检查时再推进请求，直到完成。调用提交并不代表读写已经结束
Owner使用SDIO接口执行Device提出的动作，SDIO Card再通过下方的SDIOHost控制器那里完成实际传输

以“从芯片 FIFO 读一包数据”为例，两步是：

  1. 提交请求：AicOwner 调用 SdioCard 的读取接口，得到一个代表这次读取的请求对象。随后开始推进它。如果控制器还没读完，请求返回“待完成”；owner 保存这个请求，然后让出 CPU。

  2. 继续推进请求：控制器完成传输并触发中断。中断处理入口记录这个事实、唤醒 owner。owner 再把保存的请求交给 SdioCard 推进；如果完成，就取回数据并交给 AicDevice。如果还需要一步，就继续保存并等待。

  所以“两步”说的是发起操作与在稍后取得完成结果。实际执行中，第二步可能被调用多次。这样读 FIFO 的等待时间不会让驱动一直占着 CPU 空转。


  几种Ready的定义
  Aic state ready.是AIC芯片启动完成，进入日常业务
Owner progress ready：Owner此轮推进已到达可继续运行状态
SDIO操作本身返回Pending或Complete

推进一次读FIFO的过程
Owner提出提交操作，保存在Active中
操作返回Pending Owner等待
中断唤醒Owner
Owner用同一个Active操作继续调用SDIO card
若返回Complete清除Active并把结果交回核心

拆分来看，核心的IO.Pending记录自己正在等哪次SDIO请求的结果。外层Owner的Active保存这次请求的SDIO协议层的实际操作对象，两者指向同一笔业务，不是两笔并行任务。


