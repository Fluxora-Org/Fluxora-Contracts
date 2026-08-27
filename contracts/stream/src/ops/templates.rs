//! Internal, non-ABI implementations backing the `#[contractimpl]` block
//! in `lib.rs` (issue #1520). Bodies are move-only extractions of the
//! original private associated fns, now `pub(crate)` free fns taking
//! `env: &Env`; exported signatures live on the lib.rs wrappers. The
//! split axis is lifecycle operation: exactly one public impl block
//! (thin wrappers) remains in lib.rs.

use soroban_sdk::symbol_short;
use soroban_sdk::{token, Address, Env, Map, Vec};
use crate::delegation;
use crate::events;
use crate::ops::validation;
use crate::*;

pub(crate) fn register_stream_template(
    env: &Env,
    owner: Address,
    start_delay: u64,
    cliff_delay: u64,
    duration: u64,
) -> Result<u64, ContractError> {
    // Move-only extraction from `lib.rs` (issue #1520). Doc: see wrapper.
    
            owner.require_auth();
            validate_template_delays(env, start_delay, cliff_delay, duration)?;
            let ids = load_owner_template_ids(env, &owner);
            if u64::from(ids.len()) >= MAX_TEMPLATES_PER_OWNER {
                return Err(ContractError::TemplateLimitExceeded);
            }
            let active = read_active_template_count(env);
            if active >= MAX_GLOBAL_TEMPLATES {
                return Err(ContractError::TemplateLimitExceeded);
            }
            let template_id = read_next_template_id(env);
            let tpl = StreamScheduleTemplate {
                template_id,
                owner: owner.clone(),
                start_delay,
                cliff_delay,
                duration,
            };
            save_stream_template(env, &tpl);
            let mut new_ids = ids;
            new_ids.push_back(template_id);
            save_owner_template_ids(env, &owner, &new_ids);
            set_next_template_id(env, template_id + 1);
            set_active_template_count(env, active + 1);
            env.events()
                .publish((symbol_short!("tmpl_def"), template_id), tpl.clone());
            Ok(template_id)
        
}

pub(crate) fn delete_stream_template(
    env: &Env,
    owner: Address,
    template_id: u64,
) -> Result<(), ContractError> {
    // Move-only extraction from `lib.rs` (issue #1520). Doc: see wrapper.
    
            owner.require_auth();
            let tpl = load_stream_template(env, template_id)?;
            if tpl.owner != owner {
                return Err(ContractError::TemplateUnauthorized);
            }
            remove_stream_template_storage(env, template_id);
            remove_template_id_for_owner(env, &owner, template_id)?;
            let active = read_active_template_count(env);
            set_active_template_count(env, active.saturating_sub(1));
            Ok(())
        
}

pub(crate) fn get_stream_template(
    env: &Env,
    template_id: u64,
) -> Result<StreamScheduleTemplate, ContractError> {
    // Move-only extraction from `lib.rs` (issue #1520). Doc: see wrapper.
    
            load_stream_template(env, template_id)
        
}

