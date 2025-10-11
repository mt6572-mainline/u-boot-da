# u-boot-da
Boot U-Boot (or any other bare-metal binary) as DA on MediaTek MT6572

# Usage
## With preloader patcher
```
cd payload
make
cd ..
cargo r --release -- boot -i bin1 -u 0x82000000 -j 0x82000000
```
You can also specify more payloads to upload like:
```
-i bin1 bin2 -u addr1 addr2 -j jumpaddr
```
### LK
Add `-m lk` to boot payload as LK image

### Debugging preloader patcher
Run `cargo r --release -- dump-preloader` to get preloader.bin file with patches applied

## Without preloader patcher
Note that only a single binary can be uploaded, payload must have DA_ADDR base address:
```
cargo r --release -- -i bin -u 0x81e00000
```

Sometimes `send_da` fails due to preloader sending garbage data (observed on MT8135 as well). If that happens, simply reset the device.
