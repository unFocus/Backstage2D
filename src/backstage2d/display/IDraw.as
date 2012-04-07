package backstage2d.display 
{
	import flash.display.BitmapData;
	import flash.display.IBitmapDrawable;
	import flash.display.Sprite;
	import flash.geom.Matrix;
	import flash.geom.Point;
	
	/**
	 * ...
	 * @author Kevin Newman
	 */
	public interface IDraw
	{
		function draw( canvas:BitmapData, transform:Matrix = null ):BitmapData
	}
	
}
