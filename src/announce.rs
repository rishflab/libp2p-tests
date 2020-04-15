pub mod behaviour;
pub mod handler;
pub mod protocol;

use libp2p::multihash::{self, Multihash};
use serde::{de::Error, Deserialize, Deserializer, Serialize, Serializer};
use std::fmt;

#[derive(Clone, Debug, PartialOrd, Ord, PartialEq, Eq, Hash)]
pub struct SwapDigest(Multihash);

impl SwapDigest {
    pub fn new(multihash: Multihash) -> Self {
        Self(multihash)
    }
}

impl fmt::Display for SwapDigest {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", hex::encode(self.0.as_bytes()))
    }
}

impl Serialize for SwapDigest {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
        where
            S: Serializer,
    {
        let hex = hex::encode(self.0.as_bytes());

        serializer.serialize_str(&hex)
    }
}

impl<'de> Deserialize<'de> for SwapDigest {
    fn deserialize<D>(deserializer: D) -> Result<Self, <D as Deserializer<'de>>::Error>
        where
            D: Deserializer<'de>,
    {
        let hex = String::deserialize(deserializer)?;
        let bytes = hex::decode(hex).map_err(D::Error::custom)?;
        let multihash = multihash::Multihash::from_bytes(bytes).map_err(D::Error::custom)?;

        Ok(SwapDigest::new(multihash))
    }
}

#[cfg(test)]
mod tests {

    use crate::announce::{ SwapDigest, protocol::OutboundConfig, behaviour::Announce, behaviour::BehaviourOutEvent};
    use async_std;
    use futures::{pin_mut, prelude::*};
    use libp2p::{
        core::{muxing::StreamMuxer, upgrade},
        identity,
        mplex::MplexConfig,
        multihash::{Sha2_256, Multihash},
        secio::SecioConfig,
        swarm::{Swarm, SwarmEvent},
        tcp::TcpConfig,
        PeerId, Transport,
    };

    use std::{fmt, io};
    use crate::announce::behaviour::DialInformation;

    fn transport() -> (
        PeerId,
        impl Transport<
            Output = (
                PeerId,
                impl StreamMuxer<
                    Substream = impl Send,
                    OutboundSubstream = impl Send,
                    Error = impl Into<io::Error>,
                >,
            ),
            Listener = impl Send,
            ListenerUpgrade = impl Send,
            Dial = impl Send,
            Error = impl fmt::Debug,
        > + Clone,
    ) {
        let id_keys = identity::Keypair::generate_ed25519();
        let peer_id = id_keys.public().into_peer_id();
        let transport = TcpConfig::new()
            .nodelay(true)
            .upgrade(upgrade::Version::V1)
            .authenticate(SecioConfig::new(id_keys))
            .multiplex(MplexConfig::new());
        (peer_id, transport)
    }

    fn random_swap_digest() -> SwapDigest {
        SwapDigest(Sha2_256::digest(b"hello world"))
    }

    #[test]
    fn send_announce_receive_confirmation() {
        let (mut alice_swarm, alice_peer_id) = {
            let (peer_id, transport) = transport();
            let protocol = Announce::default();
            let swarm = Swarm::new(transport, protocol, peer_id.clone());
            (swarm, peer_id)
        };

        let (mut bob_swarm, bob_peer_id) = {
            let (peer_id, transport) = transport();
            let protocol = Announce::default();
            let swarm = Swarm::new(transport, protocol, peer_id.clone());
            (swarm, peer_id)
        };

        Swarm::listen_on(&mut bob_swarm, "/ip4/127.0.0.1/tcp/0".parse().unwrap()).unwrap();

        let bob_addr: libp2p::core::Multiaddr = async_std::task::block_on(async {
            loop {
                let bob_swarm_fut = bob_swarm.next_event();
                pin_mut!(bob_swarm_fut);
                match bob_swarm_fut.await {
                    SwarmEvent::NewListenAddr(addr) => return addr,
                    _ => {}
                }
            }
        });

        //alice_swarm.add_peer(bob_peer_id.clone(), bob_addr.clone());
        Swarm::dial_addr(&mut alice_swarm, bob_addr.clone());

        let send_swap_digest = random_swap_digest();
        let outbound_config = OutboundConfig::new(send_swap_digest.clone());
        
        let dial_info = DialInformation {
            peer_id: bob_peer_id.clone(),
            address_hint: Some(bob_addr.clone()),
        };

        alice_swarm.start_announce_protocol(send_swap_digest.clone(), dial_info);

        async_std::task::block_on(async move {
            loop {
                let bob_swarm_fut = bob_swarm.next_event();
                pin_mut!(bob_swarm_fut);
                match bob_swarm_fut.await {
                    SwarmEvent::Behaviour(behavior_event) => {
                        // never enters this block causing the test to hang
                        if let BehaviourOutEvent::ReceivedAnnouncement { peer, io } = behavior_event {
                            assert_eq!(io.swap_digest, send_swap_digest);
                            // assert_eq!(peer, peer)
                            return;
                        }
                    }
                    _ => {}
                }
            }
        })
    }
}
