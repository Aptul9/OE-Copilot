package com.oecopilot.app

import android.Manifest
import android.annotation.SuppressLint
import android.app.AlarmManager
import android.app.NotificationChannel
import android.app.NotificationManager
import android.app.PendingIntent
import android.content.BroadcastReceiver
import android.content.Context
import android.content.Intent
import android.content.pm.PackageManager
import android.graphics.Bitmap
import android.graphics.BitmapFactory
import android.graphics.Canvas
import android.graphics.Color
import android.graphics.Paint
import android.graphics.Path
import android.graphics.RectF
import android.graphics.Typeface
import android.os.Build
import android.webkit.CookieManager
import androidx.core.app.NotificationCompat
import androidx.core.app.NotificationManagerCompat
import androidx.core.content.ContextCompat
import androidx.work.Constraints
import androidx.work.ExistingPeriodicWorkPolicy
import androidx.work.ExistingWorkPolicy
import androidx.work.NetworkType
import androidx.work.OneTimeWorkRequest
import androidx.work.PeriodicWorkRequest
import androidx.work.WorkManager
import androidx.work.Worker
import androidx.work.WorkerParameters
import org.json.JSONArray
import org.json.JSONObject
import java.io.File
import java.io.IOException
import java.net.HttpURLConnection
import java.net.URL
import java.text.SimpleDateFormat
import java.util.Date
import java.util.Locale
import java.util.concurrent.TimeUnit

// I promemoria delle riunioni anche ad app chiusa. Un lavoro in background (ogni 15 minuti, e appena l'app va in secondo
// piano) legge impostazioni ed eventi dal sito con il cookie di sessione della WebView; il nucleo dell'app
// (src-tauri/src/promemoria.rs) dice quali allarmi programmare, quali avvisi mostrare subito e quali allarmi annullare.
// All'ora dell'allarme la notifica la fa PromemoriaAllarme. Con l'ottimizzazione della batteria tolta (Impostazioni, App,
// OE Copilot, Batteria, Senza restrizioni) il lavoro gira in orario anche a telefono fermo; gli allarmi partono comunque.

/** Il nucleo dei promemoria, nella libreria Rust dell'app */
object NucleoPromemoria {
  init {
    System.loadLibrary("oe_copilot_app_lib")
  }

  @JvmStatic external fun aggiorna(impostazioni: String, eventi: String, ora: Long, gia: String): String
}

object Promemoria {
  const val VOCE = "voce"
  private const val AZIONE = "com.oecopilot.app.PROMEMORIA"
  private const val LAVORO = "promemoria"
  private const val LAVORO_ORA = "promemoria-ora"
  private const val CANALE = "promemoria"
  private const val PREFERENZE = "promemoria"
  private const val CHIAVI = "chiavi"
  private const val GIORNO_MS = 24 * 3_600_000L
  // lato del logo o della sigla nella notifica, px
  private const val ICONA = 128
  private const val LOGHI = "loghi"

  private class SenzaSessione : Exception()

  private val rete = Constraints.Builder().setRequiredNetworkType(NetworkType.CONNECTED).build()

  // i costruttori Java e non quelli inline di Kotlin: WorkManager li compila per JVM 11, l'app per 1.8
  /** All'avvio dell'app: il lavoro ogni 15 minuti, se non c'è già */
  fun avvia(contesto: Context) {
    val lavoro = PeriodicWorkRequest.Builder(PromemoriaLavoro::class.java, 15, TimeUnit.MINUTES).setConstraints(rete).build()
    WorkManager.getInstance(contesto).enqueueUniquePeriodicWork(LAVORO, ExistingPeriodicWorkPolicy.KEEP, lavoro)
  }

  /** L'app si apre o va in secondo piano: dopo l'accesso, il permesso delle notifiche o un cambio delle impostazioni sul sito gli
   * allarmi si rifanno subito. Le letture vanno in fila, mai due insieme */
  fun aggiornaOra(contesto: Context) {
    val lavoro = OneTimeWorkRequest.Builder(PromemoriaLavoro::class.java).setConstraints(rete).build()
    WorkManager.getInstance(contesto).enqueueUniqueWork(LAVORO_ORA, ExistingWorkPolicy.APPEND_OR_REPLACE, lavoro)
  }

  /** Una lettura. Senza sessione niente promemoria; con dati vecchi (l'agente non risponde) restano gli allarmi che ci sono. Una
   * alla volta, anche fra il lavoro periodico e quello di subito */
  @Synchronized
  fun aggiorna(contesto: Context) {
    // senza il permesso delle notifiche (la richiesta può essere ancora aperta) non si decide niente: un avviso da mostrare subito
    // andrebbe perso e segnato come già visto
    if (!NotificationManagerCompat.from(contesto).areNotificationsEnabled()) return
    val sito = sito(contesto) ?: return
    try {
      val cookie = CookieManager.getInstance().getCookie(sito)
      if (cookie.isNullOrEmpty()) throw SenzaSessione()
      val impostazioni = chiama(sito, cookie, "get_promemoria", JSONArray())?.optJSONObject("promemoria")
      val adesso = System.currentTimeMillis()
      // il terzo argomento vuole anche i calendari spenti nella barra laterale: filtra solo la vista, non i promemoria
      val dati = chiama(sito, cookie, "get_events", JSONArray().put(iso(adesso)).put(iso(adesso + GIORNO_MS)).put(true)) ?: throw IOException("get_events non c'è")
      if (dati.optBoolean("stale")) return
      // sigle e loghi delle aziende, come la pagina: senza (un errore di rete) la notifica ha la sigla calcolata dal nome
      val interfaccia = try {
        chiama(sito, cookie, "get_personalizzazione", JSONArray())?.optJSONObject("interfaccia")
      } catch (e: IOException) {
        null
      } ?: JSONObject()
      pianifica(contesto, impostazioni?.toString() ?: "null", (dati.optJSONArray("events") ?: JSONArray()).toString()) { voce ->
        marca(contesto, sito, cookie, interfaccia, voce)
      }
    } catch (e: SenzaSessione) {
      pianifica(contesto, "null", "[]")
    }
  }

  /** Il nucleo decide con le chiavi della volta prima; qui si eseguono le azioni e si ricordano le chiavi nuove. Ogni voce da
   * programmare o mostrare passa prima da marca, che le aggiunge logo e sigla dell'azienda */
  @SuppressLint("ScheduleExactAlarm")
  private fun pianifica(contesto: Context, impostazioni: String, eventi: String, marca: (JSONObject) -> Unit = {}) {
    val preferenze = contesto.getSharedPreferences(PREFERENZE, Context.MODE_PRIVATE)
    val gia = JSONArray(preferenze.getStringSet(CHIAVI, emptySet())!!.toList()).toString()
    val azioni = JSONObject(NucleoPromemoria.aggiorna(impostazioni, eventi, System.currentTimeMillis(), gia))
    val allarmi = contesto.getSystemService(AlarmManager::class.java)
    val annulla = azioni.getJSONArray("annulla")
    for (i in 0 until annulla.length()) allarmi.cancel(allarme(contesto, annulla.getInt(i), null))
    val programma = azioni.getJSONArray("programma")
    for (i in 0 until programma.length()) {
      val voce = programma.getJSONObject(i).also(marca)
      val intento = allarme(contesto, voce.getInt("allarme"), voce)
      // senza il permesso degli allarmi esatti l'avviso arriva comunque, al più qualche minuto dopo
      if (Build.VERSION.SDK_INT >= 31 && !allarmi.canScheduleExactAlarms()) {
        allarmi.setAndAllowWhileIdle(AlarmManager.RTC_WAKEUP, voce.getLong("at"), intento)
      } else {
        allarmi.setExactAndAllowWhileIdle(AlarmManager.RTC_WAKEUP, voce.getLong("at"), intento)
      }
    }
    val subito = azioni.getJSONArray("subito")
    for (i in 0 until subito.length()) mostra(contesto, subito.getJSONObject(i).also(marca))
    val chiavi = azioni.getJSONArray("chiavi")
    preferenze.edit().putStringSet(CHIAVI, (0 until chiavi.length()).map { chiavi.getString(it) }.toSet()).apply()
  }

  /** L'allarme di una voce: lo stesso codice è lo stesso allarme, e un allarme programmato di nuovo prende il testo nuovo */
  private fun allarme(contesto: Context, codice: Int, voce: JSONObject?): PendingIntent {
    val intent = Intent(contesto, PromemoriaAllarme::class.java).setAction(AZIONE)
    if (voce != null) intent.putExtra(VOCE, voce.toString())
    return PendingIntent.getBroadcast(contesto, codice, intent, PendingIntent.FLAG_UPDATE_CURRENT or PendingIntent.FLAG_IMMUTABLE)
  }

  /** La notifica di una voce: una per riunione (l'anticipo più vicino prende il posto di quello di prima), sparisce quando la
   * riunione comincia, un tocco apre l'app */
  @SuppressLint("MissingPermission")
  fun mostra(contesto: Context, voce: JSONObject) {
    if (Build.VERSION.SDK_INT >= 33 && ContextCompat.checkSelfPermission(contesto, Manifest.permission.POST_NOTIFICATIONS) != PackageManager.PERMISSION_GRANTED) return
    val gestore = NotificationManagerCompat.from(contesto)
    if (Build.VERSION.SDK_INT >= 26) gestore.createNotificationChannel(NotificationChannel(CANALE, "Promemoria", NotificationManager.IMPORTANCE_HIGH))
    val apri = PendingIntent.getActivity(contesto, 0, Intent(contesto, MainActivity::class.java).addFlags(Intent.FLAG_ACTIVITY_NEW_TASK), PendingIntent.FLAG_IMMUTABLE)
    val corpo = voce.getString("corpo")
    val mancano = voce.getLong("inizio") - System.currentTimeMillis()
    val tinta = try {
      Color.parseColor(voce.optString("colore"))
    } catch (e: IllegalArgumentException) {
      Color.parseColor("#2563eb")
    }
    val logo = voce.optString("logo").takeIf { it.isNotEmpty() }?.let { BitmapFactory.decodeFile(File(File(contesto.cacheDir, LOGHI), it).path) }
    val notifica = NotificationCompat.Builder(contesto, CANALE)
      .setSmallIcon(R.drawable.ic_stat_promemoria)
      .setLargeIcon(logo?.let(::tondo) ?: sigla(tinta, voce.optString("sigla")))
      .setColor(tinta)
      .setContentTitle(voce.getString("titolo"))
      .setContentText(corpo)
      .setStyle(NotificationCompat.BigTextStyle().bigText(corpo))
      .setCategory(NotificationCompat.CATEGORY_EVENT)
      .setPriority(NotificationCompat.PRIORITY_HIGH)
      .setContentIntent(apri)
      .setAutoCancel(true)
      .apply { if (mancano > 0) setTimeoutAfter(mancano) }
      .build()
    gestore.notify(voce.getInt("notifica"), notifica)
  }

  /** Logo (il nome del file nella cache) e sigla dell'azienda di una voce, come logoDi e avviso in inizializzazione.js: la notifica
   * può partire da un allarme a telefono senza rete, il logo va scaricato prima */
  private fun marca(contesto: Context, sito: String, cookie: String, interfaccia: JSONObject, voce: JSONObject) {
    val azienda = voce.optString("azienda")
    val indirizzo = interfaccia.optJSONObject("loghi")?.optString(azienda)?.takeIf { it.isNotEmpty() }
      ?: if (azienda == "Eventi personali") "/logos/personale.png" else null
    indirizzo?.let { logo(contesto, sito, cookie, it) }?.let { voce.put("logo", it) }
    voce.put("sigla", interfaccia.optJSONObject("sigle")?.optString(azienda)?.takeIf { it.isNotEmpty() } ?: iniziali(azienda))
  }

  /** Un logo del sito (mai un indirizzo di fuori) nella cache dell'app, scaricato una volta: un logo nuovo ha un indirizzo nuovo
   * ("logos/roar.png?v=..."). Il nome del file, o null se non c'è o non è un'immagine: allora la sigla, e si ritenta al giro dopo */
  private fun logo(contesto: Context, sito: String, cookie: String, indirizzo: String): String? {
    try {
      val base = URL("$sito/")
      val url = URL(base, indirizzo)
      if (url.protocol != base.protocol || url.authority != base.authority) return null
      val nome = Integer.toHexString(url.toString().hashCode())
      val file = File(File(contesto.cacheDir, LOGHI), nome)
      if (file.exists()) return nome
      val connessione = url.openConnection() as HttpURLConnection
      try {
        connessione.instanceFollowRedirects = false
        connessione.connectTimeout = 20_000
        connessione.readTimeout = 30_000
        connessione.setRequestProperty("Cookie", cookie)
        if (connessione.responseCode !in 200..299) return null
        val dati = connessione.inputStream.use { it.readBytes() }
        val misure = BitmapFactory.Options().apply { inJustDecodeBounds = true }
        BitmapFactory.decodeByteArray(dati, 0, dati.size, misure)
        // al più 1 MB: lo decodifica il ricevitore dell'allarme
        if (dati.size > 1_000_000 || misure.outWidth <= 0) return null
        file.parentFile?.mkdirs()
        file.writeBytes(dati)
        return nome
      } finally {
        connessione.disconnect()
      }
    } catch (e: IOException) {
      return null
    }
  }

  /** Il logo dentro un disco bianco, come nell'avviso a comparsa del PC */
  private fun tondo(immagine: Bitmap): Bitmap {
    val fuori = Bitmap.createBitmap(ICONA, ICONA, Bitmap.Config.ARGB_8888)
    val tela = Canvas(fuori)
    val pennello = Paint(Paint.ANTI_ALIAS_FLAG or Paint.FILTER_BITMAP_FLAG).apply { color = Color.WHITE }
    tela.drawCircle(ICONA / 2f, ICONA / 2f, ICONA / 2f, pennello)
    tela.clipPath(Path().apply { addCircle(ICONA / 2f, ICONA / 2f, ICONA / 2f, Path.Direction.CW) })
    val margine = ICONA * 0.1f
    tela.drawBitmap(immagine, null, RectF(margine, margine, ICONA - margine, ICONA - margine), pennello)
    return fuori
  }

  /** La sigla dell'azienda nel suo colore, come nel calendario, dove l'azienda non ha logo */
  private fun sigla(tinta: Int, testo: String): Bitmap {
    val fuori = Bitmap.createBitmap(ICONA, ICONA, Bitmap.Config.ARGB_8888)
    val tela = Canvas(fuori)
    val pennello = Paint(Paint.ANTI_ALIAS_FLAG).apply { color = tinta }
    tela.drawCircle(ICONA / 2f, ICONA / 2f, ICONA / 2f, pennello)
    pennello.color = Color.WHITE
    pennello.textAlign = Paint.Align.CENTER
    pennello.textSize = ICONA * 0.4f
    pennello.typeface = Typeface.DEFAULT_BOLD
    tela.drawText(testo, ICONA / 2f, ICONA / 2f - (pennello.ascent() + pennello.descent()) / 2, pennello)
    return fuori
  }

  /** Come iniziali in inizializzazione.js: "Alfa" -> "AL", "Beta Servizi" -> "BS" */
  private fun iniziali(nome: String): String {
    val s = nome.split("·")[0].trim().split("@")[0]
    val parole = s.split(Regex("[\\s._\\-+]")).filter { it.isNotEmpty() }
    return (if (parole.size >= 2) "${parole[0][0]}${parole[1][0]}" else s.take(2)).uppercase()
  }

  /** L'indirizzo del sito: quello scelto nell'app (indirizzo.txt nella cartella dei dati, scritto da indirizzo.rs) e, senza, quello che la
   * build mette nell'app (plugins.oe.url in assets/tauri.conf.json) */
  private fun sito(contesto: Context): String? = try {
    val scelto = File(contesto.dataDir, "indirizzo.txt").takeIf { it.isFile }?.readText()?.trim()?.trimEnd('/')
    if (scelto != null && (scelto.startsWith("https://") || scelto.startsWith("http://"))) {
      scelto
    } else {
      val configurazione = contesto.assets.open("tauri.conf.json").bufferedReader().use { it.readText() }
      JSONObject(configurazione).getJSONObject("plugins").getJSONObject("oe").getString("url").trimEnd('/')
    }
  } catch (e: Exception) {
    null
  }

  /** Una chiamata all'API come quella della pagina; null se il metodo non c'è (404). Senza sessione (401, un reindirizzamento,
   * una pagina al posto del JSON) SenzaSessione, gli altri errori IOException */
  private fun chiama(sito: String, cookie: String, metodo: String, argomenti: JSONArray): JSONObject? {
    val connessione = URL("$sito/api/$metodo").openConnection() as HttpURLConnection
    try {
      connessione.requestMethod = "POST"
      connessione.instanceFollowRedirects = false
      connessione.connectTimeout = 20_000
      connessione.readTimeout = 30_000
      connessione.doOutput = true
      connessione.setRequestProperty("Content-Type", "application/json")
      connessione.setRequestProperty("Cookie", cookie)
      connessione.outputStream.use { it.write(argomenti.toString().toByteArray()) }
      val stato = connessione.responseCode
      if (stato == 404) return null
      if (stato == 401 || stato in 300..399) throw SenzaSessione()
      if (stato !in 200..299) throw IOException("HTTP $stato")
      if (connessione.contentType?.contains("application/json") != true) throw SenzaSessione()
      return JSONObject(connessione.inputStream.bufferedReader().use { it.readText() })
    } finally {
      connessione.disconnect()
    }
  }

  /** Come la pagina: ISO con lo scarto orario del telefono */
  private fun iso(ms: Long): String = SimpleDateFormat("yyyy-MM-dd'T'HH:mm:ssXXX", Locale.US).format(Date(ms))
}

/** Il lavoro in background: una lettura. Un errore di rete si riprova più tardi, il resto aspetta il giro dopo */
class PromemoriaLavoro(contesto: Context, parametri: WorkerParameters) : Worker(contesto, parametri) {
  override fun doWork(): Result = try {
    Promemoria.aggiorna(applicationContext)
    Result.success()
  } catch (e: IOException) {
    Result.retry()
  } catch (e: Exception) {
    Result.failure()
  }
}

/** All'ora di un allarme: la notifica della sua voce */
class PromemoriaAllarme : BroadcastReceiver() {
  override fun onReceive(contesto: Context, intent: Intent) {
    val voce = intent.getStringExtra(Promemoria.VOCE) ?: return
    Promemoria.mostra(contesto, JSONObject(voce))
  }
}
