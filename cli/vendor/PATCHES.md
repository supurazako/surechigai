# Vendored BLE patches

The CLI uses local copies of the `ble-peripheral-rust 0.2.0` and `btleplug
0.13.0` sources required by Linux and macOS.

- `ble-peripheral-rust`: on BlueZ, stopping an advertisement keeps the GATT
  application registered until the peripheral is dropped.
- `btleplug`: stale CoreBluetooth characteristic discovery callbacks return an
  error instead of panicking after `didModifyServices` clears the service map.

These patches can be removed after equivalent fixes are available in released
upstream crates.
