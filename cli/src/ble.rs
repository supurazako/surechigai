use anyhow::{Context, Result, anyhow, bail, ensure};
use ble_peripheral_rust::{
    Peripheral as Server, PeripheralImpl,
    gatt::{
        characteristic::Characteristic as ServerCharacteristic,
        peripheral_event::{
            PeripheralEvent, ReadRequestResponse, RequestResponse, WriteRequestResponse,
        },
        properties::{AttributePermission, CharacteristicProperty},
        service::Service,
    },
};
use btleplug::{
    api::{
        Central, CentralEvent, CentralState, CharPropFlags, Characteristic, Manager as _,
        Peripheral as _, ScanFilter, WriteType,
    },
    platform::{Adapter, Manager, Peripheral},
};
use futures::StreamExt;
use std::{
    collections::{HashMap, HashSet},
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};
use surechigai::{
    config::{Config, Role, rssi_allowed},
    protocol::{self, ACK, Assembler, INFO, Message, RX, SELECT, SERVICE, TX},
    state::State,
};
use tokio::{
    sync::mpsc,
    task::JoinHandle,
    time::{sleep, timeout},
};
use uuid::Uuid;

const API_TIMEOUT: Duration = Duration::from_secs(10);
type SharedState = Arc<Mutex<State>>;

pub struct Radio {
    config: Config,
    state: SharedState,
    adapter: Option<Adapter>,
    server: Option<Server>,
    events: Option<JoinHandle<()>>,
    connected: Option<Peripheral>,
    known_peers: HashMap<String, Uuid>,
    scanning: bool,
}

impl Radio {
    pub fn new(config: Config) -> Self {
        let own = Message {
            node: Uuid::new_v4(),
            text: config.message.clone(),
        };
        println!(
            "自分のID={} メッセージ={:?} RSSI閾値={}dBm",
            own.node, own.text, config.rssi_threshold
        );
        let state = Arc::new(Mutex::new(State::new(
            own,
            Duration::from_secs(config.exchange_timeout_secs),
            Duration::from_secs(config.cooldown_secs),
        )));
        Self {
            config,
            state,
            adapter: None,
            server: None,
            events: None,
            connected: None,
            known_peers: HashMap::new(),
            scanning: false,
        }
    }

    async fn initialize(&mut self) -> Result<()> {
        if self.config.role != Role::Peripheral {
            let manager = Manager::new().await.context("BLE接続側の初期化に失敗")?;
            self.adapter = Some(
                manager
                    .adapters()
                    .await?
                    .into_iter()
                    .next()
                    .context("BLEアダプターが見つかりません")?,
            );
            let adapter = self.adapter.as_ref().unwrap();
            println!("接続側アダプター: {}", adapter.adapter_info().await?);
            loop {
                match adapter.adapter_state().await? {
                    CentralState::PoweredOn => break,
                    CentralState::PoweredOff => {
                        bail!("BluetoothがOFFです。OSの設定で有効にしてください")
                    }
                    CentralState::Unknown => sleep(Duration::from_millis(100)).await,
                }
            }
        }
        if self.config.role != Role::Central {
            let (sender, receiver) = mpsc::channel(256);
            self.server = Some(
                Server::new(sender)
                    .await
                    .context("BLE待受側の初期化に失敗（Bluetooth権限を確認してください）")?,
            );
            self.events = Some(tokio::spawn(serve_events(receiver, self.state.clone())));
            let server = self.server.as_mut().unwrap();
            while !server.is_powered().await? {
                sleep(Duration::from_millis(100)).await;
            }
            server
                .add_service(&Service {
                    uuid: SERVICE,
                    primary: true,
                    characteristics: vec![
                        server_characteristic(INFO, false),
                        server_characteristic(RX, true),
                        server_characteristic(TX, false),
                    ],
                })
                .await
                .context("GATTサービスの登録に失敗")?;
        }
        Ok(())
    }

    pub async fn run(&mut self) -> Result<()> {
        timeout(Duration::from_secs(20), self.initialize())
            .await
            .context(
                "BLE初期化がタイムアウトしました。Bluetoothの電源・権限を確認してください",
            )??;
        let mut role = match self.config.role {
            Role::Auto if rand::random() => Role::Central,
            Role::Auto => Role::Peripheral,
            role => role,
        };
        loop {
            if self.events.as_ref().is_some_and(|task| task.is_finished()) {
                bail!("待受イベント処理が停止しました");
            }
            self.state.lock().unwrap().expire(Instant::now());
            let duration = self.config.slot_duration();
            println!("役割={role:?} 継続時間={}秒", duration.as_secs());
            let result = match role {
                Role::Peripheral => self.peripheral_slot(duration).await,
                Role::Central => self.central_slot(duration).await,
                Role::Auto => unreachable!(),
            };
            if let Err(error) = result {
                eprintln!("通信失敗: {error:#}");
                // Do not start another role until previous radio operations are stopped.
                self.stop_activity().await?;
                sleep(self.config.slot_duration()).await;
            }
            if self.config.role == Role::Auto {
                role = if role == Role::Central {
                    Role::Peripheral
                } else {
                    Role::Central
                };
            }
        }
    }

    async fn peripheral_slot(&mut self, duration: Duration) -> Result<()> {
        let server = self.server.as_mut().context("待受アダプターがありません")?;
        timeout(
            API_TIMEOUT,
            server.start_advertising("surechigai", &[SERVICE]),
        )
        .await
        .context("広告開始タイムアウト")??;
        self.state.lock().unwrap().enable();
        sleep(duration).await;
        loop {
            if self.state.lock().unwrap().disable_if_idle(Instant::now()) {
                break;
            }
            sleep(Duration::from_millis(50)).await;
        }
        stop_advertising(server).await
    }

    async fn central_slot(&mut self, duration: Duration) -> Result<()> {
        let adapter = self
            .adapter
            .as_ref()
            .context("接続側アダプターがありません")?
            .clone();
        // Subscribe afresh, before scanning. Don't use the OS's cached device list.
        let mut events = adapter.events().await?;
        self.scanning = true;
        let start = timeout(
            API_TIMEOUT,
            adapter.start_scan(ScanFilter {
                services: vec![SERVICE],
            }),
        )
        .await?;
        if let Err(error) = start {
            self.scanning = false;
            return Err(error.into());
        }
        let threshold = self.config.rssi_threshold;
        let find = async {
            let mut logged = HashSet::new();
            while let Some(event) = events.next().await {
                let id = match event {
                    #[cfg(target_os = "macos")]
                    CentralEvent::ServicesAdvertisement { id, services }
                        if services.contains(&SERVICE) =>
                    {
                        id
                    }
                    // On macOS DeviceUpdated is emitted before RSSI is updated;
                    // ServicesAdvertisement is emitted afterwards. BlueZ exposes RSSI via DeviceUpdated.
                    #[cfg(not(target_os = "macos"))]
                    CentralEvent::DeviceDiscovered(id)
                    | CentralEvent::DeviceUpdated(id)
                    | CentralEvent::ServicesAdvertisement { id, .. } => id,
                    CentralEvent::StateUpdate(CentralState::PoweredOff) => {
                        bail!("BluetoothがOFFになりました")
                    }
                    _ => continue,
                };
                let peer = adapter.peripheral(&id).await?;
                let Some(properties) = peer.properties().await? else {
                    continue;
                };
                if !properties.services.contains(&SERVICE) {
                    continue;
                }
                let allowed = rssi_allowed(properties.rssi, threshold);
                let cooling_down = self.known_peers.get(&id.to_string()).is_some_and(|node| {
                    self.state
                        .lock()
                        .unwrap()
                        .cooling_down(*node, Instant::now())
                });
                if logged.insert((id.to_string(), properties.rssi)) {
                    println!(
                        "発見 device={id} RSSI={:?}dBm {}",
                        properties.rssi,
                        if cooling_down {
                            "見送り（再交換待ち）"
                        } else if allowed {
                            "接続候補"
                        } else {
                            "見送り（閾値未満またはRSSI不明）"
                        }
                    );
                }
                if allowed && !cooling_down {
                    return Ok(peer);
                }
            }
            bail!("BLEスキャンのイベントストリームが終了しました")
        };
        let found = timeout(duration, find).await;
        self.stop_scan().await?;
        let peer = match found {
            Err(_) => return Ok(()),
            Ok(result) => result?,
        };
        self.connected = Some(peer.clone());
        let limit = Duration::from_secs(self.config.exchange_timeout_secs);
        let result = timeout(limit, exchange(&peer, self.state.clone()))
            .await
            .context("交換タイムアウト");
        // Keep the handle until disconnect succeeds, including after a cancelled connect.
        self.disconnect().await?;
        let node = result??;
        self.known_peers.insert(peer.id().to_string(), node);
        Ok(())
    }

    async fn disconnect(&mut self) -> Result<()> {
        if let Some(peer) = self.connected.as_ref() {
            timeout(Duration::from_secs(3), peer.disconnect())
                .await
                .context("切断タイムアウト")??;
            self.connected = None;
        }
        Ok(())
    }

    async fn stop_scan(&mut self) -> Result<()> {
        if self.scanning {
            let adapter = self
                .adapter
                .as_ref()
                .context("接続側アダプターがありません")?;
            timeout(API_TIMEOUT, adapter.stop_scan())
                .await
                .context("スキャン停止タイムアウト")??;
            self.scanning = false;
        }
        Ok(())
    }

    async fn stop_activity(&mut self) -> Result<()> {
        self.state.lock().unwrap().shutdown();
        let mut errors = vec![];
        if let Err(error) = self.disconnect().await {
            errors.push(format!("切断: {error:#}"));
        }
        if let Err(error) = self.stop_scan().await {
            errors.push(format!("スキャン停止: {error:#}"));
        }
        if let Some(server) = &mut self.server
            && let Err(error) = stop_advertising(server).await
        {
            errors.push(format!("広告停止: {error:#}"));
        }
        ensure!(errors.is_empty(), "終了処理に失敗: {}", errors.join("; "));
        Ok(())
    }

    pub async fn cleanup(&mut self) -> Result<()> {
        let result = self.stop_activity().await;
        if let Some(task) = self.events.take() {
            task.abort();
            let _ = task.await;
        }
        result
    }
}

async fn stop_advertising(server: &mut Server) -> Result<()> {
    timeout(API_TIMEOUT, async {
        server.stop_advertising().await?;
        while server.is_advertising().await? {
            sleep(Duration::from_millis(50)).await;
        }
        Ok::<_, anyhow::Error>(())
    })
    .await
    .context("広告停止タイムアウト")?
}

fn server_characteristic(uuid: Uuid, write: bool) -> ServerCharacteristic {
    ServerCharacteristic {
        uuid,
        properties: vec![if write {
            CharacteristicProperty::Write
        } else {
            CharacteristicProperty::Read
        }],
        permissions: vec![if write {
            AttributePermission::Writeable
        } else {
            AttributePermission::Readable
        }],
        value: None,
        descriptors: vec![],
    }
}

async fn serve_events(mut events: mpsc::Receiver<PeripheralEvent>, state: SharedState) {
    let mut tick = tokio::time::interval(Duration::from_millis(100));
    loop {
        tokio::select! {
            _ = tick.tick() => state.lock().unwrap().expire(Instant::now()),
            event = events.recv() => {
                let Some(event) = event else { return; };
                match event {
                    PeripheralEvent::ReadRequest { request, offset, responder } => {
                        let result = if request.service != SERVICE || offset != 0 {
                            Err(anyhow!("invalid service or offset"))
                        } else {
                            state.lock().unwrap().read(&request.client, request.characteristic, Instant::now())
                        };
                        let response = match result {
                            Ok(value) => ReadRequestResponse { value, response: RequestResponse::Success },
                            Err(error) => {
                                eprintln!("読出見送り: {error:#}");
                                ReadRequestResponse { value: vec![], response: RequestResponse::RequestNotSupported }
                            }
                        };
                        let _ = responder.send(response);
                    }
                    PeripheralEvent::WriteRequest { request, offset, value, responder } => {
                        let result = if request.service != SERVICE || offset != 0 {
                            Err(anyhow!("invalid service or offset"))
                        } else {
                            state.lock().unwrap().write(&request.client, request.characteristic, &value, Instant::now())
                        };
                        let response = match result {
                            Ok(()) => RequestResponse::Success,
                            Err(error) => {
                                eprintln!("書込見送り: {error:#}");
                                RequestResponse::RequestNotSupported
                            }
                        };
                        let _ = responder.send(WriteRequestResponse { response });
                    }
                    PeripheralEvent::StateUpdate { is_powered } => {
                        println!("待受側Bluetooth powered={is_powered}");
                        if !is_powered { state.lock().unwrap().shutdown(); }
                    }
                    PeripheralEvent::CharacteristicSubscriptionUpdate { .. } => (),
                }
            }
        }
    }
}

fn characteristic(
    peer: &Peripheral,
    uuid: Uuid,
    property: CharPropFlags,
) -> Result<Characteristic> {
    peer.characteristics()
        .into_iter()
        .find(|c| c.service_uuid == SERVICE && c.uuid == uuid && c.properties.contains(property))
        .with_context(|| format!("必要なCharacteristicがありません: {uuid}"))
}

async fn exchange(peer: &Peripheral, state: SharedState) -> Result<Uuid> {
    peer.connect().await.context("BLE接続に失敗")?;
    peer.discover_services()
        .await
        .context("サービス探索に失敗")?;
    let info = characteristic(peer, INFO, CharPropFlags::READ)?;
    let rx = characteristic(peer, RX, CharPropFlags::WRITE)?;
    let tx = characteristic(peer, TX, CharPropFlags::READ)?;
    let node = protocol::parse_identity(&peer.read(&info).await?)?;
    let own = {
        let state = state.lock().unwrap();
        ensure!(node != state.own.node, "自分自身への接続です");
        if state.cooling_down(node, Instant::now()) {
            println!("見送り peer={node}（再交換待ち）");
            return Ok(node);
        }
        state.own.clone()
    };
    let exchange = rand::random::<u32>();
    println!("交換開始 role=central peer={node} exchange={exchange:08x}");
    for frame in own.frames(exchange)? {
        peer.write(&rx, &frame, WriteType::WithResponse)
            .await
            .context("メッセージ書込に失敗")?;
    }
    let mut assembler = Assembler::default();
    // A 146-byte envelope occupies at most 13 frames (12 payload bytes each).
    for index in 0..13 {
        let mut select = protocol::command(SELECT, exchange);
        select.push(index);
        peer.write(&rx, &select, WriteType::WithResponse).await?;
        let frame = peer.read(&tx).await.context("返信読出に失敗")?;
        protocol::require_exchange(&frame, exchange)?;
        if let Some(message) = assembler.push(&frame)? {
            ensure!(message.node == node, "相手IDが交換途中で変わりました");
            peer.write(
                &rx,
                &protocol::command(ACK, exchange),
                WriteType::WithResponse,
            )
            .await
            .context("受信確認に失敗")?;
            state.lock().unwrap().record(&message, Instant::now());
            return Ok(node);
        }
    }
    bail!("返信のフレーム数が上限を超えました")
}
