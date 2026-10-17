package com.oecopilot.app

import android.os.Bundle
import android.view.View
import android.webkit.CookieManager
import androidx.activity.enableEdgeToEdge
import androidx.core.view.ViewCompat
import androidx.core.view.WindowInsetsCompat

class MainActivity : TauriActivity() {
  override fun onCreate(savedInstanceState: Bundle?) {
    enableEdgeToEdge()
    super.onCreate(savedInstanceState)
    // Il sito non conosce le barre di sistema (niente safe-area-inset): senza questo l'intestazione finisce sotto la barra
    // di stato. La vista sta tra le barre e si accorcia quando compare la tastiera.
    ViewCompat.setOnApplyWindowInsetsListener(findViewById<View>(android.R.id.content)) { vista, insets ->
      val spazio = insets.getInsets(
        WindowInsetsCompat.Type.systemBars() or WindowInsetsCompat.Type.displayCutout() or WindowInsetsCompat.Type.ime()
      )
      vista.setPadding(spazio.left, spazio.top, spazio.right, spazio.bottom)
      WindowInsetsCompat.CONSUMED
    }
    // i promemoria delle riunioni anche ad app chiusa (Promemoria.kt)
    Promemoria.avvia(this)
  }

  override fun onResume() {
    super.onResume()
    // all'apertura dell'app, e quando si chiude la richiesta del permesso delle notifiche, gli allarmi si rifanno subito
    Promemoria.aggiornaOra(this)
  }

  override fun onPause() {
    super.onPause()
    // La WebView scrive i cookie su disco ogni 30 secondi circa: se il sistema chiude l'app prima, il login (cookie di
    // sessione) si perde. Si scrivono quando l'app passa in secondo piano.
    CookieManager.getInstance().flush()
    // dopo l'accesso o un cambio delle impostazioni dei promemoria sul sito, gli allarmi si rifanno subito
    Promemoria.aggiornaOra(this)
  }
}
