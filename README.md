Boot U-Boot (or any other bare-metal binary) as DA

Usage:
  - With preloader patcher:
    - `cd payload`
    - `make`
    - `cd ..`
    - `cargo r --release -- -i bin1 -u 0x82000000 -j 0x82000000` (you can specify more payloads to upload like `-i bin1 bin2 -u addr1 addr2 -j jumpaddr`)
  - Without preloader patcher (only one binary can be uploaded, payload must have DA_ADDR base address): `cargo r --release -- -i bin -u 0x81e00000`

Sometimes send_da fails due to preloader sending garbage data (seems to be a MediaTek issue, observed on the MT8135 aswell), in that case simply reset the device.
