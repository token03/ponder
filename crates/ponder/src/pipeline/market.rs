use super::{
    context::{Context, Progress},
    types::Estimate,
};
use crate::gear::Prices;

#[derive(Clone, Copy)]
enum Quote {
    Market,
    Base,
}

fn lookup(cx: &Context, estimate: &mut Estimate, table: &Prices, league: &str, quote: Quote) {
    let query = match quote {
        Quote::Market => &estimate.query,
        Quote::Base => &estimate.base_query,
    };
    let Some(query) = query else { return };
    let (price, error) = match cx.trade.price(league, query, table, 10, None) {
        Ok(Some(price)) => (Some(price), None),
        Ok(None) => (None, Some("No listings".into())),
        Err(error) => (None, Some(error.to_string())),
    };
    match quote {
        Quote::Market => {
            estimate.market = price;
            estimate.market_error = error;
        }
        Quote::Base => {
            estimate.base_price = price;
            estimate.base_error = error;
            estimate.base_source = if price.is_some() { "trade".into() } else { String::new() };
        }
    }
}

pub fn lookup_market(cx: &Context, estimate: &mut Estimate, table: &Prices, league: &str) {
    for quote in [Quote::Market, Quote::Base] {
        lookup(cx, estimate, table, league, quote);
    }
}

pub fn price_on_trade(
    cx: &Context,
    estimates: &mut [Estimate],
    order: &[usize],
    table: &Prices,
    league: &str,
    budget: usize,
    progress: Option<Progress>,
) -> usize {
    let before = cx.trade.searches();
    for &index in order {
        if let Some(say) = progress {
            say(&format!("trade: {}", estimates[index].title));
        }
        for quote in [Quote::Market, Quote::Base] {
            if cx.trade.searches() - before >= budget {
                break;
            }
            lookup(cx, &mut estimates[index], table, league, quote);
        }
        if cx.trade.searches() - before >= budget {
            break;
        }
    }
    cx.trade.searches() - before
}
