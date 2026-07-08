use sqlx::PgPool;
use tokio::sync::mpsc;
use uuid::Uuid;

use crate::scraper::registry::ScraperRegistry;

/// Commands that can be sent to the job worker
#[derive(Debug)]
pub enum JobCommand {
    /// Run a specific scrape job by ID
    RunJob(Uuid),
    /// Shutdown the worker gracefully
    Shutdown,
}

/// Background worker that processes scrape jobs
pub struct JobWorker {
    receiver: mpsc::Receiver<JobCommand>,
    db: PgPool,
    registry: ScraperRegistry,
}

impl JobWorker {
    /// Create a new job worker and return it along with a sender for dispatching commands
    pub fn new(
        db: PgPool,
        registry: ScraperRegistry,
    ) -> (Self, mpsc::Sender<JobCommand>) {
        let (sender, receiver) = mpsc::channel(32);
        let worker = Self {
            receiver,
            db,
            registry,
        };
        (worker, sender)
    }

    /// Run the worker event loop, processing jobs until shutdown
    pub async fn run(&mut self) {
        tracing::info!("Job worker started");
        while let Some(cmd) = self.receiver.recv().await {
            match cmd {
                JobCommand::RunJob(job_id) => {
                    tracing::info!("Processing scrape job: {}", job_id);
                    if let Err(e) = self.process_job(job_id).await {
                        tracing::error!("Job {} failed: {}", job_id, e);
                    }
                }
                JobCommand::Shutdown => {
                    tracing::info!("Job worker shutting down");
                    break;
                }
            }
        }
    }

    /// Process a single scrape job
    async fn process_job(&self, job_id: Uuid) -> Result<(), Box<dyn std::error::Error>> {
        use crate::models::scrape_job::ScrapeJob;
        use crate::models::scrape_source::ScrapeSource;

        // 1. Fetch job and source from DB
        let job: ScrapeJob = sqlx::query_as("SELECT * FROM scrape_jobs WHERE id = $1")
            .bind(job_id)
            .fetch_one(&self.db)
            .await?;

        let source: ScrapeSource = sqlx::query_as("SELECT * FROM scrape_sources WHERE id = $1")
            .bind(job.source_id)
            .fetch_one(&self.db)
            .await?;

        // 2. Update job status to 'running'
        sqlx::query(
            "UPDATE scrape_jobs SET status = 'running', started_at = NOW() WHERE id = $1",
        )
        .bind(job_id)
        .execute(&self.db)
        .await?;

        // 3. Get scraper from registry
        let scraper = match self.registry.get(&source.platform) {
            Some(s) => s,
            None => {
                // No scraper registered for this platform
                sqlx::query(
                    "UPDATE scrape_jobs SET status = 'failed', error_message = $1, completed_at = NOW() WHERE id = $2",
                )
                .bind(format!("No scraper registered for platform: {}", source.platform))
                .bind(job_id)
                .execute(&self.db)
                .await?;
                return Ok(());
            }
        };

        // 4. Call scraper
        match scraper
            .scrape(&source.source_url, &source.scrape_config)
            .await
        {
            Ok(products) => {
                let items_found = products.len() as i32;

                // 5. Store results in scrape_results
                for product in &products {
                    let raw_data = serde_json::to_value(product)
                        .map_err(|e| format!("Failed to serialize product: {}", e))?;

                    sqlx::query(
                        "INSERT INTO scrape_results (job_id, raw_data) VALUES ($1, $2)",
                    )
                    .bind(job_id)
                    .bind(&raw_data)
                    .execute(&self.db)
                    .await?;
                }

                // 6. Update job status to 'completed'
                sqlx::query(
                    "UPDATE scrape_jobs SET status = 'completed', items_found = $1, completed_at = NOW() WHERE id = $2",
                )
                .bind(items_found)
                .bind(job_id)
                .execute(&self.db)
                .await?;

                // 7. Update source last_scraped_at
                sqlx::query(
                    "UPDATE scrape_sources SET last_scraped_at = NOW() WHERE id = $1",
                )
                .bind(source.id)
                .execute(&self.db)
                .await?;

                tracing::info!(
                    "Job {} completed: {} items found",
                    job_id,
                    items_found
                );
            }
            Err(e) => {
                // Update job status to 'failed'
                sqlx::query(
                    "UPDATE scrape_jobs SET status = 'failed', error_message = $1, completed_at = NOW() WHERE id = $2",
                )
                .bind(e.to_string())
                .bind(job_id)
                .execute(&self.db)
                .await?;

                tracing::error!("Job {} failed with error: {}", job_id, e);
            }
        }

        Ok(())
    }
}
