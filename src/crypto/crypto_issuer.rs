use ursa::cl::{
  CredentialPublicKey,
  CredentialPrivateKey,
  new_nonce,
  RevocationKeyPrivate,
  SimpleTailsAccessor,
  RevocationRegistryDelta,
  RevocationRegistry,
};
use ursa::cl::issuer::Issuer as CryptoIssuer;
use crate::crypto::crypto_datatypes::{
  CryptoCredentialRequest,
  CryptoCredentialDefinition,
  SignedCredential,
  RevocationRegistryDefinition
};
use crate::application::datatypes::{
  CredentialSchema
};

pub struct Issuer {
}

impl Issuer {

  pub fn new() -> Issuer {
    Issuer {
    }
  }

  pub fn create_credential_definition(
    credential_schema: &CredentialSchema,
    include_master_secret: bool
  ) -> (CredentialPrivateKey, CryptoCredentialDefinition) {
    let mut non_credential_schema_builder = CryptoIssuer::new_non_credential_schema_builder().unwrap();
    if include_master_secret {
      non_credential_schema_builder.add_attr("master_secret").unwrap();
    }
    let non_credential_schema = non_credential_schema_builder.finalize().unwrap();

    // Retrieve property names from schema
    // TODO: Object handling, how to handle nested object properties?
    let mut credential_schema_builder = CryptoIssuer::new_credential_schema_builder().unwrap();
    for property in &credential_schema.properties {
      credential_schema_builder.add_attr(&property.0).unwrap();
    }
    let crypto_schema = credential_schema_builder.finalize().unwrap();

    let (public_key, credential_private_key, credential_key_correctness_proof) =
      CryptoIssuer::new_credential_def(&crypto_schema, &non_credential_schema, false).unwrap();

    let definition = CryptoCredentialDefinition {
      public_key,
      credential_key_correctness_proof
    };

    return (credential_private_key, definition);
  }

  /**
   * Creates a new ISO credential schema and stores it in the registry.
   * Returns the ID of the schema in the registry.
   */
  // pub fn create_credential_schema(
  //   schema_name: String,
  //   author: String,
  //   description: String,
  //   properties: HashMap<String, SchemaProperty>,
  //   required: Vec<String>,
  //   allow_additional_properties: bool,
  //   author_private_key: String
  // ) -> Result<CredentialSchema, Box<dyn std::error::Error>> {
  //   let created_at = Issuer::get_timestamp_now();
  //   let did = Issuer::get_new_did();
  //   let schema_type = "EvanVCSchema".to_string();
  //   let mut schema = CredentialSchema {
  //     id: did,
  //     name: schema_name,
  //     author,
  //     created_at,
  //     description,
  //     properties,
  //     required,
  //     r#type: schema_type,
  //     additional_properties: allow_additional_properties,
  //     proof: None
  //   };

  //   let doc_to_sign = serde_json::to_value(schema).unwrap();
  //   let proof_val = Issuer::create_proof(&doc_to_sign, "?", &author, &author_private_key).unwrap();
  //   let proof : AssertionProof = serde_json::from_value(proof_val).unwrap();
  //   schema.proof = Some(proof);

  //   return Ok(schema);
  // }

  pub fn sign_credential(
    credential_request: &CryptoCredentialRequest,
    credential_private_key: CredentialPrivateKey,
    credential_public_key: CredentialPublicKey
  ) -> SignedCredential {
    let credential_issuance_nonce = new_nonce().unwrap();

    let (cred, proof) = CryptoIssuer::sign_credential(&credential_request.subject,
                              &credential_request.blinded_credential_secrets,
                              &credential_request.blinded_credential_secrets_correctness_proof,
                              &credential_request.credential_nonce,
                              &credential_issuance_nonce,
                              &credential_request.credential_values,
                              &credential_public_key,
                              &credential_private_key).unwrap();
    return SignedCredential {
      signature: cred,
      correctness_proof: proof,
      issuance_nonce: credential_issuance_nonce
    }
  }

  pub fn sign_credential_with_revocation(
    credential_request: &CryptoCredentialRequest,
    credential_private_key: &CredentialPrivateKey,
    credential_public_key: &CredentialPublicKey,
    credential_revocation_definition: &mut RevocationRegistryDefinition,
    credential_revocation_id: u32,
    revocation_private_key: &RevocationKeyPrivate
  ) -> SignedCredential {
    let credential_issuance_nonce = new_nonce().unwrap();

    let tails_accessor = SimpleTailsAccessor::new(&mut credential_revocation_definition.tails).unwrap();

    // no delta because we assume issuance_by_default ==true
    let (cred, proof, _) = CryptoIssuer::sign_credential_with_revoc(
      &credential_request.subject,
      &credential_request.blinded_credential_secrets,
      &credential_request.blinded_credential_secrets_correctness_proof,
      &credential_request.credential_nonce,
      &credential_issuance_nonce,
      &credential_request.credential_values,
      credential_public_key,
      credential_private_key,
      credential_revocation_id,
      credential_revocation_definition.maximum_credential_count,
      true, // TODO: Make global var
      &mut credential_revocation_definition.registry,
      &revocation_private_key,
      &tails_accessor
    ).unwrap();

    return SignedCredential {
      signature: cred,
      correctness_proof: proof,
      issuance_nonce: credential_issuance_nonce
    };
  }

  pub fn create_revocation_registry(
    credential_public_key: &CredentialPublicKey,
    maximum_credential_count: u32
  ) -> (RevocationRegistryDefinition, RevocationKeyPrivate) {
    let (rev_key_pub, rev_key_priv, rev_registry, rev_tails_gen) = CryptoIssuer::new_revocation_registry_def(
      credential_public_key,
      maximum_credential_count,
      true
    ).unwrap();

    let rev_def = RevocationRegistryDefinition {
      registry: rev_registry,
      registry_delta: None,
      tails: rev_tails_gen,
      revocation_public_key: rev_key_pub,
      maximum_credential_count
    };

    return (rev_def, rev_key_priv);
  }

  pub fn update_revocation_registry(revocation_registry_delta: RevocationRegistryDelta) -> RevocationRegistry {
    let new_registry = RevocationRegistry::from(revocation_registry_delta);
    return new_registry;
  }
}
