Boot U-Boot (or any other bare-metal binary) as DA

Usage: `cargo r --release -- path/to/bin`

Sometimes send_da fails due to preloader sending garbage data (seems to be a MediaTek issue, observed on the MT8135 aswell), in that case simply reset the device.
