use std::collections::HashMap;

use anyhow::{anyhow, Result};
use reqwest::{Client, ClientBuilder, Response};
use serde::de::DeserializeOwned;

use crate::quote::{InternalQuoteRequest, QuoteRequest, QuoteResponse};
use crate::swap::{SwapInstructionsResponse, SwapInstructionsResponseInternal, SwapRequest, SwapResponse};

#[derive(Clone)]
pub struct JupiterSwapApiClient {
    pub base_path: String,
    pub client: Client,
}

async fn check_is_success(response: Response) -> Result<Response> {
    if !response.status().is_success() {
        return Err(anyhow!(
            "Request status not ok: {}, body: {:?}",
            response.status(),
            response.text().await
        ));
    }
    Ok(response)
}

async fn check_status_code_and_deserialize<T: DeserializeOwned>(response: Response) -> Result<T> {
    check_is_success(response)
        .await?
        .json::<T>()
        .await
        .map_err(Into::into)
}

impl JupiterSwapApiClient {
    pub fn new(base_path: String, api_key: &'static str) -> Self {
        use reqwest::header;
        let mut headers = header::HeaderMap::new();
        headers.insert("x-api-key", header::HeaderValue::from_static(api_key));
        let client = ClientBuilder::new()
            .default_headers(headers)
            .build()
            .unwrap();

        Self { base_path, client }
    }

    /// Fetches a swap quote from the Jupiter aggregator.
    ///
    /// This method queries the Jupiter API to get the best available swap route and pricing
    /// information for exchanging one Solana token for another. The quote includes the expected
    /// output amount, price impact, and the route through various DEXes.
    ///
    /// # Arguments
    ///
    /// * `quote_request` - The quote request containing:
    ///   - `input_mint`: The mint address of the token to swap from
    ///   - `output_mint`: The mint address of the token to swap to
    ///   - `amount`: The amount to swap in the smallest unit
    ///   - `slippage_bps`: Maximum allowed slippage in basis points
    ///   - Additional optional parameters for routing preferences
    ///
    /// # Returns
    ///
    /// Returns a `QuoteResponse` containing:
    /// - The estimated output amount
    /// - Price impact percentage
    /// - Detailed route plan with swap steps
    /// - Slippage settings
    /// - Platform fees (if any)
    ///
    /// # Errors
    ///
    /// This function will return an error if:
    /// - The API request fails (network issues, API downtime)
    /// - The response cannot be deserialized
    /// - The requested token pair has no liquidity
    /// - Invalid parameters are provided
    ///
    /// # Example
    ///
    /// ```ignore
    /// use jupiter_swap_api_client::{JupiterSwapApiClient, quote::QuoteRequest};
    /// use solana_sdk::pubkey::Pubkey;
    /// use std::str::FromStr;
    ///
    /// #[tokio::main]
    /// async fn main() -> Result<(), Box<dyn std::error::Error>> {
    ///     let client = JupiterSwapApiClient::new(
    ///         "https://api.jup.ag/swap/v1".to_string(),
    ///         "your_api_key"
    ///     );
    ///
    ///     let quote_request = QuoteRequest {
    ///         input_mint: Pubkey::from_str("So11111111111111111111111111111111111111112")?,
    ///         output_mint: Pubkey::from_str("EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v")?,
    ///         amount: 1_000_000_000, // 1 SOL (9 decimals)
    ///         slippage_bps: 50, // 0.5% slippage
    ///         ..Default::default()
    ///     };
    ///
    ///     let quote = client.quote(&quote_request).await?;
    ///     println!("Output amount: {}", quote.out_amount);
    ///     Ok(())
    /// }
    /// ```
    pub async fn quote(&self, quote_request: &QuoteRequest) -> Result<QuoteResponse> {
        let url = format!("{}/quote", self.base_path);
        let extra_args = quote_request.quote_args.clone();
        let internal_quote_request = InternalQuoteRequest::from(quote_request.clone());
        let response = self
            .client
            .get(url)
            .query(&internal_quote_request)
            .query(&extra_args)
            .send()
            .await?;
        let res = check_status_code_and_deserialize(response).await;
        tracing::debug!("Jupiter Deserialized Response: {res:?}");
        res
    }

    /// Executes a token swap on the Solana blockchain.
    ///
    /// This method creates and returns a transaction for performing the actual token swap
    /// based on a previously obtained quote. The transaction is not submitted to the blockchain;
    /// it needs to be signed and sent by the caller.
    ///
    /// # Arguments
    ///
    /// * `swap_request` - The swap request containing:
    ///   - `user_public_key`: The user's wallet public key
    ///   - `quote_response`: A valid quote response from the `quote` method
    ///   - Transaction configuration options (priority fees, compute units, etc.)
    ///
    /// * `extra_args` - Optional HashMap of additional query parameters for the API request, such
    ///   as custom routing preferences or advanced options not covered in SwapRequest
    ///
    /// # Returns
    ///
    /// Returns a `SwapResponse` containing:
    /// - `swap_transaction`: The serialized versioned transaction ready to be signed and sent
    /// - `last_valid_block_height`: The block height until which the transaction is valid
    /// - Additional metadata about the swap
    ///
    /// # Errors
    ///
    /// This function will return an error if:
    /// - The API request fails
    /// - The quote has expired or is invalid
    /// - The user has insufficient balance
    /// - Transaction construction fails
    /// - Response deserialization fails
    ///
    /// # Example
    ///
    /// ```ignore
    /// use jupiter_swap_api_client::{JupiterSwapApiClient, swap::SwapRequest};
    /// use solana_sdk::pubkey::Pubkey;
    /// use std::str::FromStr;
    ///
    /// #[tokio::main]
    /// async fn main() -> Result<(), Box<dyn std::error::Error>> {
    ///     let client = JupiterSwapApiClient::new(
    ///         "https://api.jup.ag/swap/v1".to_string(),
    ///         "your_api_key"
    ///     );
    ///
    ///     // First get a quote
    ///     let quote = client.quote(&quote_request).await?;
    ///
    ///     // Then create a swap transaction
    ///     let swap_request = SwapRequest {
    ///         user_public_key: user_pubkey,
    ///         quote_response: quote,
    ///         ..Default::default()
    ///     };
    ///
    ///     let swap = client.swap(&swap_request, None).await?;
    ///     // Sign and send swap.swap_transaction
    ///     Ok(())
    /// }
    /// ```
    pub async fn swap(
        &self,
        swap_request: &SwapRequest,
        extra_args: Option<HashMap<String, String>>,
    ) -> Result<SwapResponse> {
        let response = self
            .client
            .post(format!("{}/swap", self.base_path))
            .query(&extra_args)
            .json(swap_request)
            .send()
            .await?;
        check_status_code_and_deserialize(response).await
    }

    /// Retrieves the raw swap instructions without constructing a transaction.
    ///
    /// This method returns the individual Solana instructions needed to perform a swap,
    /// allowing for custom transaction construction. This is useful when you need to
    /// combine the swap with other operations in a single transaction or need fine-grained
    /// control over transaction assembly.
    ///
    /// # Arguments
    ///
    /// * `swap_request` - The swap request containing:
    ///   - `user_public_key`: The user's wallet public key
    ///   - `quote_response`: A valid quote response from the `quote` method
    ///   - Configuration options for the swap
    ///
    /// # Returns
    ///
    /// Returns a `SwapInstructionsResponse` containing:
    /// - `token_ledger_instruction`: Optional instruction for token ledger operations
    /// - `compute_budget_instructions`: Instructions for setting compute budget
    /// - `setup_instructions`: Instructions to set up token accounts if needed
    /// - `swap_instruction`: The main swap instruction
    /// - `cleanup_instruction`: Optional instruction to close temporary accounts
    /// - `address_lookup_table_addresses`: Addresses of lookup tables used
    ///
    /// # Errors
    ///
    /// This function will return an error if:
    /// - The API request fails
    /// - The quote is invalid or expired
    /// - Instruction generation fails
    /// - Response deserialization fails
    ///
    /// # Example
    ///
    /// ```ignore
    /// use jupiter_swap_api_client::{JupiterSwapApiClient, swap::SwapRequest};
    /// use solana_sdk::{
    ///     pubkey::Pubkey,
    ///     transaction::Transaction,
    ///     message::Message,
    /// };
    ///
    /// #[tokio::main]
    /// async fn main() -> Result<(), Box<dyn std::error::Error>> {
    ///     let client = JupiterSwapApiClient::new(
    ///         "https://api.jup.ag/swap/v1".to_string(),
    ///         "your_api_key"
    ///     );
    ///
    ///     // Get a quote first
    ///     let quote = client.quote(&quote_request).await?;
    ///
    ///     // Get swap instructions
    ///     let swap_request = SwapRequest {
    ///         user_public_key: user_pubkey,
    ///         quote_response: quote,
    ///         ..Default::default()
    ///     };
    ///
    ///     let instructions = client.swap_instructions(&swap_request).await?;
    ///
    ///     // Now you can build a custom transaction with these instructions
    ///     // combined with your own additional instructions
    ///     Ok(())
    /// }
    /// ```
    pub async fn swap_instructions(
        &self,
        swap_request: &SwapRequest,
    ) -> Result<SwapInstructionsResponse> {
        let response = self
            .client
            .post(format!("{}/swap-instructions", self.base_path))
            .json(swap_request)
            .send()
            .await?;
        check_status_code_and_deserialize::<SwapInstructionsResponseInternal>(response)
            .await
            .map(Into::into)
    }
}