use std::panic;

use bbs::{
    prelude::{DeterministicPublicKey, PublicKey},
    signature::Signature,
    HashElem,
    SignatureMessage,
};
use serde_json::{value::Value, Map};
use ssi::{
    jsonld::{json_to_dataset, JsonLdOptions, StaticLoader},
    urdna2015::normalize,
};
use vade_evan_bbs::{
    BbsCredential,
    CredentialSchema,
    CredentialSchemaReference,
    CredentialStatus,
    CredentialSubject,
    OfferCredentialPayload,
    UnsignedBbsCredential,
};

use crate::api::{VadeEvan, VadeEvanError};
use crate::datatypes::DidDocument;

// Master secret is always incorporated, without being mentioned in the credential schema
const ADDITIONAL_HIDDEN_MESSAGES_COUNT: usize = 1;
const EVAN_METHOD: &str = "did:evan";
const TYPE_OPTIONS: &str = r#"{ "type": "bbs" }"#;

fn create_empty_unsigned_credential(
    schema_did_doc_str: &str,
    subject_did: Option<&str>,
    use_valid_until: bool,
) -> Result<UnsignedBbsCredential, VadeEvanError> {
    let response_obj: Value = serde_json::from_str(&schema_did_doc_str)?;
    let did_document_obj = response_obj.get("didDocument").ok_or_else(|| {
        VadeEvanError::InvalidDidDocument("missing 'didDocument' in response".to_string())
    });
    let did_document_str = serde_json::to_string(&did_document_obj?)?;
    let schema_obj: CredentialSchema = serde_json::from_str(&did_document_str)?;

    let credential = UnsignedBbsCredential {
        context: vec![
            "https://www.w3.org/2018/credentials/v1".to_string(),
            "https://schema.org/".to_string(),
            "https://w3id.org/vc-revocation-list-2020/v1".to_string(),
        ],
        id: "uuid:834ca9da-9f09-4359-8264-c890de13cdc8".to_string(),
        r#type: vec!["VerifiableCredential".to_string()],
        issuer: "did:evan:testcore:placeholder_issuer".to_string(),
        valid_until: if use_valid_until {
            Some("2031-01-01T00:00:00.000Z".to_string())
        } else {
            None
        },
        issuance_date: "2021-01-01T00:00:00.000Z".to_string(),
        credential_subject: CredentialSubject {
            id: subject_did.map(|s| s.to_owned()), // subject.id stays optional, defined by create_offer call
            data: schema_obj // fill ALL subject data fields with empty string (mandatory and optional ones)
                .properties
                .into_iter()
                .map(|(name, _schema_property)| (name, String::new()))
                .collect(),
        },
        credential_schema: CredentialSchemaReference {
            id: schema_obj.id,
            r#type: schema_obj.r#type,
        },
        credential_status: CredentialStatus {
            id: "did:evan:zkp:placeholder_status#0".to_string(),
            r#type: "RevocationList2020Status".to_string(),
            revocation_list_index: "0".to_string(),
            revocation_list_credential: "did:evan:zkp:placeholder_status".to_string(),
        },
    };

    Ok(credential)
}

async fn convert_to_nquads(document_string: &str) -> Result<Vec<String>, VadeEvanError> {
    let mut loader = StaticLoader;
    let options = JsonLdOptions {
        base: None,           // -b, Base IRI
        expand_context: None, // -c, IRI for expandContext option
        ..Default::default()
    };
    let dataset = json_to_dataset(
        &document_string,
        None, // will be patched into @context, e.g. Some(&r#"["https://schema.org/"]"#.to_string()),
        false,
        Some(&options),
        &mut loader,
    )
    .await
    .map_err(|err| VadeEvanError::JsonLdHandling(err.to_string()))?;
    let dataset_normalized = normalize(&dataset).unwrap();
    let normalized = dataset_normalized.to_nquads().unwrap();
    let non_empty_lines = normalized
        .split("\n")
        .filter(|s| !s.is_empty())
        .map(|s| s.to_string())
        .collect();

    Ok(non_empty_lines)
}

fn get_public_key_generator(
    public_key: &str,
    message_count: usize,
) -> Result<PublicKey, VadeEvanError> {
    let public_key: DeterministicPublicKey =
        DeterministicPublicKey::from(base64::decode(public_key)?.into_boxed_slice());
    let public_key_generator = public_key.to_public_key(message_count).map_err(|e| {
        VadeEvanError::PublicKeyParsingError(format!(
            "public key invalid, generate public key generator; {}",
            e
        ))
    })?;

    Ok(public_key_generator)
}

pub struct Credential<'a> {
    vade_evan: &'a mut VadeEvan,
}

impl<'a> Credential<'a> {
    pub fn new(vade_evan: &'a mut VadeEvan) -> Result<Credential, VadeEvanError> {
        Ok(Credential { vade_evan })
    }

    pub async fn create_credential_offer(
        self,
        schema_did: &str,
        use_valid_until: bool,
        issuer_did: &str,
        subject_did: Option<&str>,
    ) -> Result<String, VadeEvanError> {
        let schema_did_doc_str = self.vade_evan.did_resolve(schema_did).await?;

        let credential_draft = create_empty_unsigned_credential(
            &schema_did_doc_str,
            subject_did.as_deref(),
            use_valid_until,
        )?;
        let credential_draft_str = serde_json::to_string(&credential_draft)?;
        let nquads = convert_to_nquads(&credential_draft_str).await?;

        let payload = OfferCredentialPayload {
            issuer: issuer_did.to_string(),
            subject: subject_did.map(|v| v.to_string()),
            nquad_count: nquads.len(),
        };
        let result = self
            .vade_evan
            .vc_zkp_create_credential_offer(
                EVAN_METHOD,
                TYPE_OPTIONS,
                &serde_json::to_string(&payload)?,
            )
            .await?;

        Ok(result)
    }

    /// Resolve a issuer did, get the did document and extract the public key out of the
    /// verification methods
    ///
    /// # Arguments
    /// * `issuer_did` - DID of the issuer to load the pub key from
    /// * `verification_method_id` - id of verification method to extract the pub key
    ///
    /// # Returns
    /// * `CredentialProposal` - The message to be sent to an issuer
    async fn get_issuer_public_key(
        &mut self,
        issuer_did: &str,
        verification_method_id: &str,
    ) -> Result<String, VadeEvanError> {
        // resolve the did and extract the did document out of it
        let did_result_str = self.vade_evan.did_resolve(issuer_did).await?;
        let did_result_value: Value = serde_json::from_str(&did_result_str)?;
        let did_document_result = did_result_value.get("didDocument").ok_or_else(|| {
            VadeEvanError::InvalidDidDocument(
                "missing 'didDocument' property in resolved did".to_string(),
            )
        });
        let did_document_str = serde_json::to_string(&did_document_result?)?;
        let did_document: DidDocument = serde_json::from_str(&did_document_str)?;

        // get the verification methods
        let verification_methods =
            did_document
                .verification_method
                .ok_or(VadeEvanError::InvalidVerificationMethod(
                    "missing 'verification_method' property in did_document".to_string(),
                ))?;

        let mut public_key: &str = "";
        for method in verification_methods.iter() {
            if method.id == verification_method_id {
                public_key = &method.public_key_jwk.x;
                break;
            }
        }

        if public_key == "" {
            return Err(VadeEvanError::InvalidVerificationMethod(format!(
                "no public key found for verification id {}",
                &verification_method_id
            )));
        }

        Ok(public_key.to_string())
    }

    async fn verify_proof_signature(
        &self,
        signature: &str,
        did_doc_nquads: &Vec<String>,
        master_secret: &str,
        pk: &PublicKey,
    ) -> Result<(), VadeEvanError> {
        let mut signature_messages: Vec<SignatureMessage> = Vec::new();
        let master_secret_message: SignatureMessage =
            SignatureMessage::from(base64::decode(master_secret)?.into_boxed_slice());
        signature_messages.insert(0, master_secret_message);
        let mut i = 1;
        for message in did_doc_nquads {
            signature_messages.insert(i, SignatureMessage::hash(message));
            i += 1;
        }
        let decoded_proof = base64::decode(signature)?;
        // let signature = Signature::from(Box::from(decoded_proof.as_slice()));
        let signature = Signature::from(decoded_proof.into_boxed_slice());
        let is_valid = signature
            .verify(&signature_messages, &pk)
            .map_err(|err| VadeEvanError::BbsValidationError(err.to_string()))?;

        dbg!(&is_valid);

        Ok(())
    }

    pub async fn verify_credential(
        &mut self,
        credential_str: &str,
        verification_method_id: &str,
        master_secret: &str,
    ) -> Result<(), VadeEvanError> {
        let credential: BbsCredential = serde_json::from_str(credential_str)?;

        // get nquads
        let mut parsed_credential: Map<String, Value> = serde_json::from_str(credential_str)?;
        parsed_credential.remove("proof");
        let credential_without_proof = serde_json::to_string(&parsed_credential)?;
        let did_doc_nquads = convert_to_nquads(&credential_without_proof).await?;

        // get public key suitable for messages
        let issuer_pub_key = self
            .get_issuer_public_key(&credential.issuer, verification_method_id)
            .await?;
        let public_key_generator = get_public_key_generator(
            &issuer_pub_key,
            did_doc_nquads.len() + ADDITIONAL_HIDDEN_MESSAGES_COUNT,
        )?;

        // verify signature
        self.verify_proof_signature(
            &credential.proof.signature,
            &did_doc_nquads,
            master_secret,
            &public_key_generator,
        )
        .await?;

        // TODO: check if credential has not been revoked?

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use anyhow::Result;
    use vade_evan_bbs::BbsCredentialOffer;

    use crate::{VadeEvan, DEFAULT_SIGNER, DEFAULT_TARGET};

    use super::Credential;

    const CREDENTIAL_MESSAGE_COUNT: usize = 13;
    const VALID_ISSUER_DID: &str = "did:evan:EiAee4ixDnSP0eWyp0YFV7Wt9yrZ3w841FNuv9NSLFSCVA";
    const NON_EXISTING_ISSUER_DID: &str =
        "did:evan:testcore:0x6240cedfc840579b7fdcd686bdc65a9a8c42dea6";
    const SCHEMA_DID: &str = "did:evan:EiACv4q04NPkNRXQzQHOEMa3r1p_uINgX75VYP2gaK5ADw";
    const SUBJECT_DID: &str = "did:evan:testcore:0x67ce8b01b3b75a9ba4a1462139a1edaa0d2f539f";
    const VERIFICATION_METHOD_ID: &str = "#bbs-key-1";
    const JSON_WEB_PUB_KEY: &str = "qWZ7EGhzYsSlBq4mLhNal6cHXBD88ZfncdbEWQoue6SaAbZ7k56IxsjcvuXD6LGYDgMgtjTHnBraaMRiwJVBJenXgOT8nto7ZUTO/TvCXwtyPMzGrLM5JNJdEaPP4QJN";
    const MASTER_SECRET: &str = "QyRmu33oIQFNW+dSI5wex3u858Ra7yx5O1tsxJgQvu8=";
    const EXAMPLE_CREDENTIAL: &str = r###"{
        "id": "uuid:70b7ec4e-f035-493e-93d3-2cf5be4c7f88",
        "type": [
            "VerifiableCredential"
        ],
        "proof": {
            "type": "BbsBlsSignature2020",
            "created": "2023-02-01T14:08:17.000Z",
            "signature": "kvSyi40dnZ5S3/mSxbSUQGKLpyMXDQNLCPtwDGM9GsnNNKF7MtaFHXIbvXaVXku0EY/n2uNMQ2bmK2P0KEmzgbjRHtzUOWVdfAnXnVRy8/UHHIyJR471X6benfZk8KG0qVqy+w67z9g628xRkFGA5Q==",
            "proofPurpose": "assertionMethod",
            "verificationMethod": "did:evan:EiAee4ixDnSP0eWyp0YFV7Wt9yrZ3w841FNuv9NSLFSCVA#bbs-key-1",
            "credentialMessageCount": 13,
            "requiredRevealStatements": []
        },
        "issuer": "did:evan:EiAee4ixDnSP0eWyp0YFV7Wt9yrZ3w841FNuv9NSLFSCVA",
        "@context": [
            "https://www.w3.org/2018/credentials/v1",
            "https://schema.org/",
            "https://w3id.org/vc-revocation-list-2020/v1"
        ],
        "issuanceDate": "2023-02-01T14:08:09.849Z",
        "credentialSchema": {
            "id": "did:evan:EiCimsy3uWJ7PivWK0QUYSCkImQnjrx6fGr6nK8XIg26Kg",
            "type": "EvanVCSchema"
        },
        "credentialStatus": {
            "id": "did:evan:EiA0Ns-jiPwu2Pl4GQZpkTKBjvFeRXxwGgXRTfG1Lyi8aA#4",
            "type": "RevocationList2020Status",
            "revocationListIndex": "4",
            "revocationListCredential": "did:evan:EiA0Ns-jiPwu2Pl4GQZpkTKBjvFeRXxwGgXRTfG1Lyi8aA"
        },
        "credentialSubject": {
            "id": "did:evan:EiAee4ixDnSP0eWyp0YFV7Wt9yrZ3w841FNuv9NSLFSCVA",
            "data": {
                "bio": "biography"
            }
        }
    }"###;

    #[tokio::test]
    #[cfg(not(all(feature = "target-c-lib", feature = "capability-sdk")))]
    async fn helper_can_create_credential_offer() -> Result<()> {
        let mut vade_evan = VadeEvan::new(crate::VadeEvanConfig {
            target: DEFAULT_TARGET,
            signer: DEFAULT_SIGNER,
        })?;
        let credential = Credential::new(&mut vade_evan)?;

        let offer_str = credential
            .create_credential_offer(SCHEMA_DID, false, VALID_ISSUER_DID, Some(SUBJECT_DID))
            .await?;

        let offer_obj: BbsCredentialOffer = serde_json::from_str(&offer_str)?;
        assert_eq!(offer_obj.issuer, VALID_ISSUER_DID);
        assert_eq!(offer_obj.subject, Some(SUBJECT_DID.to_string()));
        assert_eq!(offer_obj.credential_message_count, CREDENTIAL_MESSAGE_COUNT);
        assert!(!offer_obj.nonce.is_empty());

        Ok(())
    }

    #[tokio::test]
    #[cfg(not(all(feature = "target-c-lib", feature = "capability-sdk")))]
    async fn helper_can_verify_credential() -> Result<()> {
        let mut vade_evan = VadeEvan::new(crate::VadeEvanConfig {
            target: DEFAULT_TARGET,
            signer: DEFAULT_SIGNER,
        })?;

        let mut credential = Credential::new(&mut vade_evan)?;

        // TODO: verify credential nquads

        // verify the credential issuer
        credential
            .verify_credential(EXAMPLE_CREDENTIAL, VERIFICATION_METHOD_ID, MASTER_SECRET)
            .await?;

        Ok(())
    }

    #[tokio::test]
    #[cfg(not(all(feature = "target-c-lib", feature = "capability-sdk")))]
    async fn can_get_issuer_pub_key() -> Result<()> {
        let mut vade_evan = VadeEvan::new(crate::VadeEvanConfig {
            target: DEFAULT_TARGET,
            signer: DEFAULT_SIGNER,
        })?;

        let mut credential = Credential::new(&mut vade_evan)?;
        let pub_key = credential
            .get_issuer_public_key(VALID_ISSUER_DID, VERIFICATION_METHOD_ID)
            .await?;

        assert_eq!(pub_key, JSON_WEB_PUB_KEY);

        Ok(())
    }

    #[tokio::test]
    #[cfg(not(all(feature = "target-c-lib", feature = "capability-sdk")))]
    async fn will_throw_when_pubkey_not_found() -> Result<()> {
        let mut vade_evan = VadeEvan::new(crate::VadeEvanConfig {
            target: DEFAULT_TARGET,
            signer: DEFAULT_SIGNER,
        })?;

        let mut credential = Credential::new(&mut vade_evan)?;
        let pub_key = credential
            .get_issuer_public_key(VALID_ISSUER_DID, "#random-id")
            .await;

        match pub_key {
            Ok(_) => assert!(false, "pub key should not be there"),
            Err(_) => assert!(true, "pub key not found"),
        }

        Ok(())
    }
}
