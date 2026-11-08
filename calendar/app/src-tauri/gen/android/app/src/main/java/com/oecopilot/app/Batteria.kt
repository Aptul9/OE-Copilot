package com.oecopilot.app

import android.annotation.SuppressLint
import android.app.Activity
import android.content.ActivityNotFoundException
import android.content.Intent
import android.net.Uri
import android.os.PowerManager
import android.provider.Settings
import android.webkit.JavascriptInterface

// L'ottimizzazione della batteria per la pagina del sito (window.OeBatteria, inizializzazione.js): con l'ottimizzazione attiva Android
// rimanda o salta il lavoro dei promemoria ad app chiusa (Promemoria.kt), e la pagina lo dice con una fascia rossa finché non si toglie.
// Su HyperOS "Nessuna restrizione" nella batteria dell'app vale come ottimizzazione tolta (l'app entra nella whitelist di deviceidle).
class Batteria(private val activity: Activity) {
  @JavascriptInterface
  fun ottimizzata(): Boolean =
    !activity.getSystemService(PowerManager::class.java).isIgnoringBatteryOptimizations(activity.packageName)

  // La richiesta di Android per togliere l'ottimizzazione a questa app; dove manca, l'elenco delle app ottimizzate
  @SuppressLint("BatteryLife")
  @JavascriptInterface
  fun togli() {
    activity.runOnUiThread {
      try {
        activity.startActivity(Intent(Settings.ACTION_REQUEST_IGNORE_BATTERY_OPTIMIZATIONS, Uri.parse("package:${activity.packageName}")))
      } catch (e: ActivityNotFoundException) {
        activity.startActivity(Intent(Settings.ACTION_IGNORE_BATTERY_OPTIMIZATION_SETTINGS))
      }
    }
  }
}
