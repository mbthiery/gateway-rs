use crate::{
    service::{CONNECT_TIMEOUT, RPC_TIMEOUT},
    Error, KeyedUri, MsgSign, MsgVerify, Region, Result,
};
use helium_crypto::{Keypair, PublicKey};
use helium_proto::{
    gateway_resp_v1,
    services::{self, Channel, Endpoint},
    BlockchainTxnStateChannelCloseV1, BlockchainVarV1, GatewayConfigReqV1, GatewayConfigRespV1,
    GatewayRegionParamsUpdateReqV1, GatewayRespV1, GatewayRoutingReqV1, GatewayScCloseReqV1,
    GatewayScFollowReqV1, GatewayScFollowStreamedRespV1, GatewayScIsActiveReqV1,
    GatewayScIsActiveRespV1, GatewayValidatorsReqV1, GatewayValidatorsRespV1, Routing,
};
use rand::{rngs::OsRng, seq::SliceRandom};
use std::{sync::Arc, time::Duration};
use tokio::sync::mpsc;
use tokio_stream::wrappers::ReceiverStream;

type GatewayClient = services::gateway::Client<Channel>;

#[derive(Debug)]
pub struct Streaming {
    streaming: tonic::Streaming<GatewayRespV1>,
    verifier: Arc<PublicKey>,
}

#[derive(Debug, Clone)]
pub struct Response(GatewayRespV1);

impl Streaming {
    pub async fn message(&mut self) -> Result<Option<Response>> {
        match self.streaming.message().await {
            Ok(Some(response)) => {
                response.verify(&self.verifier)?;
                Ok(Some(Response(response)))
            }
            Ok(None) => Ok(None),
            Err(err) => Err(err.into()),
        }
    }
}

impl Response {
    pub fn height(&self) -> u64 {
        self.0.height
    }

    pub fn routings(&self) -> Result<&[Routing]> {
        match &self.0.msg {
            Some(gateway_resp_v1::Msg::RoutingStreamedResp(routings)) => Ok(&routings.routings),
            msg => Err(Error::custom(
                format!("Unexpected gateway message {msg:?}",),
            )),
        }
    }

    pub fn region(&self) -> Result<Region> {
        match &self.0.msg {
            Some(gateway_resp_v1::Msg::RegionParamsStreamedResp(params)) => {
                Region::from_i32(params.region)
            }
            msg => Err(Error::custom(
                format!("Unexpected gateway message {msg:?}",),
            )),
        }
    }
}

#[derive(Debug)]
pub struct StateChannelFollowService {
    tx: mpsc::Sender<GatewayScFollowReqV1>,
    rx: Streaming,
}

impl StateChannelFollowService {
    pub async fn new(mut client: GatewayClient, verifier: Arc<PublicKey>) -> Result<Self> {
        let (tx, client_rx) = mpsc::channel(3);
        let streaming = client
            .follow_sc(ReceiverStream::new(client_rx))
            .await?
            .into_inner();
        let rx = Streaming {
            streaming,
            verifier,
        };
        Ok(Self { tx, rx })
    }

    pub async fn send(&mut self, id: &[u8], owner: &[u8]) -> Result {
        let msg = GatewayScFollowReqV1 {
            sc_id: id.into(),
            sc_owner: owner.into(),
        };
        Ok(self.tx.send(msg).await?)
    }

    pub async fn message(&mut self) -> Result<Option<GatewayScFollowStreamedRespV1>> {
        use helium_proto::gateway_resp_v1::Msg;
        match self.rx.message().await {
            Ok(Some(Response(GatewayRespV1 {
                msg: Some(Msg::FollowStreamedResp(resp)),
                ..
            }))) => Ok(Some(resp)),
            Ok(None) => Ok(None),
            Ok(msg) => Err(Error::custom(format!(
                "unexpected gateway response {msg:?}",
            ))),
            Err(err) => Err(err),
        }
    }
}

#[derive(Debug, Clone)]
pub struct GatewayService {
    pub uri: KeyedUri,
    client: GatewayClient,
}

impl GatewayService {
    pub fn new(keyed_uri: KeyedUri) -> Result<Self> {
        let channel = Endpoint::from(keyed_uri.uri.clone())
            .connect_timeout(Duration::from_secs(CONNECT_TIMEOUT))
            .timeout(Duration::from_secs(RPC_TIMEOUT))
            .connect_lazy();
        Ok(Self {
            uri: keyed_uri,
            client: GatewayClient::new(channel),
        })
    }

    pub async fn random_new(seed_uris: &[KeyedUri]) -> Result<Self> {
        let seed_uri = seed_uris
            .choose(&mut OsRng)
            .ok_or_else(|| Error::custom("empty uri list"))?;
        let mut seed_gateway = Self::new(seed_uri.to_owned())?;
        let mut gateways = seed_gateway.validators(1).await?;
        gateways
            .pop()
            .ok_or_else(|| Error::custom("empty gateway list"))
            .and_then(Self::new)
    }

    pub async fn routing(&mut self, height: u64) -> Result<Streaming> {
        let stream = self.client.routing(GatewayRoutingReqV1 { height }).await?;
        Ok(Streaming {
            streaming: stream.into_inner(),
            verifier: self.uri.pubkey.clone(),
        })
    }

    pub async fn region_params(&mut self, keypair: Arc<Keypair>) -> Result<Streaming> {
        let mut req = GatewayRegionParamsUpdateReqV1 {
            address: keypair.public_key().to_vec(),
            signature: vec![],
        };
        req.signature = req.sign(keypair).await?;

        let stream = self.client.region_params_update(req).await?;
        Ok(Streaming {
            streaming: stream.into_inner(),
            verifier: self.uri.pubkey.clone(),
        })
    }

    pub async fn is_active_sc(
        &mut self,
        id: &[u8],
        owner: &[u8],
    ) -> Result<GatewayScIsActiveRespV1> {
        let resp = self
            .client
            .is_active_sc(GatewayScIsActiveReqV1 {
                sc_owner: owner.into(),
                sc_id: id.into(),
            })
            .await?
            .into_inner();
        resp.verify(&self.uri.pubkey)?;
        match resp.msg {
            Some(gateway_resp_v1::Msg::IsActiveResp(resp)) => {
                let GatewayScIsActiveRespV1 {
                    sc_id, sc_owner, ..
                } = &resp;
                if sc_id == id && sc_owner == owner {
                    Ok(resp)
                } else {
                    Err(Error::custom("mismatched state channel id and owner"))
                }
            }
            Some(other) => Err(Error::custom(format!(
                "invalid is_active response {other:?}",
            ))),
            None => Err(Error::custom("empty is_active response")),
        }
    }

    pub async fn follow_sc(&mut self) -> Result<StateChannelFollowService> {
        StateChannelFollowService::new(self.client.clone(), self.uri.pubkey.clone()).await
    }

    pub async fn close_sc(&mut self, close_txn: BlockchainTxnStateChannelCloseV1) -> Result {
        let _ = self
            .client
            .close_sc(GatewayScCloseReqV1 {
                close_txn: Some(close_txn),
            })
            .await?;
        Ok(())
    }

    async fn get_config(&mut self, keys: Vec<String>) -> Result<GatewayRespV1> {
        let resp = self
            .client
            .config(GatewayConfigReqV1 { keys })
            .await?
            .into_inner();
        resp.verify(&self.uri.pubkey)?;
        Ok(resp)
    }

    pub async fn config(&mut self, keys: Vec<String>) -> Result<Vec<BlockchainVarV1>> {
        match self.get_config(keys).await?.msg {
            Some(gateway_resp_v1::Msg::ConfigResp(GatewayConfigRespV1 { result })) => Ok(result),
            Some(other) => Err(Error::custom(format!("invalid config response {other:?}"))),
            None => Err(Error::custom("empty config response")),
        }
    }

    pub async fn height(&mut self) -> Result<(u64, u64)> {
        let resp = self.get_config(vec![]).await?;
        Ok((resp.height, resp.block_age))
    }

    pub async fn validators(&mut self, quantity: u32) -> Result<Vec<KeyedUri>> {
        let resp = self
            .client
            .validators(GatewayValidatorsReqV1 { quantity })
            .await?
            .into_inner();
        resp.verify(&self.uri.pubkey)?;
        match resp.msg {
            Some(gateway_resp_v1::Msg::ValidatorsResp(GatewayValidatorsRespV1 { result })) => {
                result.into_iter().map(KeyedUri::try_from).collect()
            }
            Some(other) => Err(Error::custom(format!(
                "invalid validator response {other:?}"
            ))),
            None => Err(Error::custom("empty validator response")),
        }
    }
}
