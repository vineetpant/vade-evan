/*
  Copyright (c) 2018-present evan GmbH.

  Licensed under the Apache License, Version 2.0 (the "License");
  you may not use this file except in compliance with the License.
  You may obtain a copy of the License at

      http://www.apache.org/licenses/LICENSE-2.0

  Unless required by applicable law or agreed to in writing, software
  distributed under the License is distributed on an "AS IS" BASIS,
  WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
  See the License for the specific language governing permissions and
  limitations under the License.
*/

extern crate vade;
use async_trait::async_trait;
use vade::traits::{ DidResolver, MessageConsumer };
use crate::utils::substrate::{
    get_did,
    create_did,
    add_payload_to_did,
    get_payload_count_for_did,
    update_payload_in_did,
    whitelist_identity
};


pub struct ResolverConfig {
  pub target: String,
  pub private_key: String,
  pub identity: Vec<u8>
}

/// Resolver for DIDs on evan.network (currently on testnet)
pub struct SubstrateDidResolverEvan {
  config: ResolverConfig
}

impl SubstrateDidResolverEvan {
    /// Creates new instance of `SubstrateDidResolverEvan`.
    pub fn new(config: ResolverConfig) -> SubstrateDidResolverEvan {
        SubstrateDidResolverEvan {
          config
        }
    }

    async fn generate_did(&self) -> Result<Option<String>, Box<dyn std::error::Error>> {
        Ok(Some(create_did(self.config.target.clone(), self.config.private_key.clone(), self.config.identity.clone()).await.unwrap()))
    }

    async fn whitelist_identity(&self) -> Result<Option<String>, Box<dyn std::error::Error>> {
        Ok(Some(whitelist_identity(self.config.target.clone(), self.config.private_key.clone(), self.config.identity.clone()).await.unwrap()))
    }
}

#[async_trait(?Send)]
impl DidResolver for SubstrateDidResolverEvan {
    /// Checks given DID document.
    /// A DID document is considered as valid if returning ().
    /// Resolver may throw to indicate
    /// - that it is not responsible for this DID
    /// - that it considers this DID as invalid
    ///
    /// Currently the test `did_name` `"test"` is accepted as valid.
    ///
    /// # Arguments
    ///
    /// * `did_name` - did_name to check document for
    /// * `value` - value to check
    async fn check_did(&self, _did_name: &str, _value: &str) -> Result<(), Box<dyn std::error::Error>> {
        unimplemented!();
    }

    /// Gets document for given did name.
    ///
    /// # Arguments
    ///
    /// * `did_id` - did id to fetch
    async fn get_did_document(&self, did_id: &str) -> Result<String, Box<dyn std::error::Error>> {
        let didresult = get_did(self.config.target.clone(), did_id.to_string()).await;
        Ok(didresult.unwrap())
    }

    /// Sets document for given did name.
    ///
    /// # Arguments
    ///
    /// * `did_name` - did_name to set value for
    /// * `value` - value to set
    async fn set_did_document(&mut self, did_id: &str, value: &str) -> std::result::Result<(), Box<dyn std::error::Error>> {
        let payload_count: u32 = get_payload_count_for_did(self.config.target.clone(), did_id.to_string()).await.unwrap();
        if payload_count > 0 {
            update_payload_in_did(self.config.target.clone(), 0 as u32, value.to_string(), did_id.to_string(), self.config.private_key.clone(), self.config.identity.clone()).await.unwrap();
        } else {
            add_payload_to_did(self.config.target.clone(), value.to_string(), did_id.to_string(), self.config.private_key.clone(), self.config.identity.clone()).await.unwrap();
        }
        Ok(())
    }
}

#[async_trait(?Send)]
impl MessageConsumer for SubstrateDidResolverEvan {
    /// Reacts to `Vade` messages.
    ///
    /// # Arguments
    ///
    /// * `message_data` - arbitrary data for plugin, e.g. a JSON
    async fn handle_message(
        &mut self,
        message_type: &str,
        _message_data: &str,
    ) -> Result<Option<String>, Box<dyn std::error::Error>> {
        match message_type {
            "generateDid" => self.generate_did().await,
            "whitelistIdentity" => self.whitelist_identity().await,
            _ => Err(Box::from(format!("message type '{}' not implemented", message_type)))
        }
    }
}
